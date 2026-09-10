//! The bounded tool loop (`mush_llm::agent`, S3) against a `MockProvider`: the
//! answered-only echo across a round, the one repair, the round bound, the per-run
//! cost cap checked before a call, a declined call, and a service refusal.

#![cfg(feature = "mock")]

use std::sync::Arc;

use mush_llm::agent::{
    RunBudget, RunLimits, RunSpec, StopCause, TextVerdict, ToolHost, ToolReply, run,
};
use mush_llm::budget::BudgetLimit;
use mush_llm::config::{BudgetConfig, RouteConfig};
use mush_llm::mock::MockProvider;
use mush_llm::provider::{LlmError, LlmProvider};
use mush_llm::service::{CallCtx, LlmFeature, LlmService, Priority};
use mush_llm::types::{
    ChatMessage, CompletionResponse, ContentBlock, Role, StopReason, ToolChoice, ToolDef, Usage,
};

fn route() -> RouteConfig {
    RouteConfig {
        model: "test/mock".into(),
        max_tokens: 512,
        timeout_s: 30,
        reasoning: None,
        reasoning_max_tokens: None,
    }
}

fn service(mock: &Arc<MockProvider>) -> LlmService {
    LlmService::builder()
        .provider("openrouter", Arc::clone(mock) as Arc<dyn LlmProvider>)
        .route("worldgen_plan", route())
        .global_enabled(true)
        .build()
}

fn tool_use(id: &str, name: &str, input: serde_json::Value) -> ContentBlock {
    ContentBlock::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    }
}

fn reply(blocks: Vec<ContentBlock>, cost: Option<f64>) -> CompletionResponse {
    CompletionResponse {
        content: blocks,
        stop_reason: StopReason::ToolUse,
        usage: Usage {
            input_tokens: 100,
            output_tokens: 20,
            cost_usd: cost,
            ..Usage::default()
        },
        model: "test/mock".into(),
    }
}

fn text(t: &str) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test/mock".into(),
    }
}

fn spec(max_rounds: usize, cap_usd: Option<f64>) -> RunSpec {
    RunSpec {
        ctx: CallCtx::new(LlmFeature::WorldgenPlan, Priority::Interactive),
        system: "You plan.".into(),
        opening: "Plan a park.".into(),
        tools: vec![ToolDef {
            name: "read".into(),
            description: "Read something.".into(),
            input_schema: serde_json::json!({ "type": "object" }),
            strict: false,
        }],
        tool_choice: ToolChoice::Auto,
        limits: RunLimits {
            max_rounds,
            budget: RunBudget {
                cap_usd,
                target_usd: None,
            },
            max_tokens: 256,
        },
    }
}

/// A host that answers `read` with "more", `submit` as the end, declines `skip`,
/// and remembers everything it was asked; `repairs` says what a text reply gets.
#[derive(Default)]
struct Host {
    calls: Vec<String>,
    texts: Vec<String>,
    submitted: bool,
    repair: Option<String>,
}

impl ToolHost for Host {
    fn call(&mut self, name: &str, input: &serde_json::Value) -> Option<ToolReply> {
        self.calls.push(format!("{name}:{input}"));
        match name {
            "read" => Some(ToolReply {
                content: "the park is 40 by 40".into(),
                is_error: false,
                more: true,
            }),
            "submit" => {
                self.submitted = true;
                Some(ToolReply {
                    content: "ok".into(),
                    is_error: false,
                    more: false,
                })
            }
            _ => None,
        }
    }

    fn on_text(&mut self, reply: &CompletionResponse) -> TextVerdict {
        self.texts.push(reply.text());
        match self.repair.take() {
            Some(prompt) => TextVerdict::Repair(prompt),
            None => TextVerdict::Done,
        }
    }

    fn finished(&self) -> bool {
        self.submitted
    }
}

#[tokio::test]
async fn a_read_round_continues_and_echoes_only_what_was_answered() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let svc = service(&mock);
    // Round 1: two reads and one declined call; round 2: the submit.
    mock.push_response(reply(
        vec![
            ContentBlock::text("Let me look."),
            tool_use("r1", "read", serde_json::json!({ "what": "site" })),
            tool_use("s1", "skip", serde_json::json!({})),
            tool_use("r2", "read", serde_json::json!({ "what": "theme" })),
        ],
        Some(0.10),
    ));
    mock.push_response(reply(
        vec![tool_use("p1", "submit", serde_json::json!({ "pack": {} }))],
        Some(0.20),
    ));
    let mut host = Host::default();
    let report = run(&svc, spec(6, None), &mut host).await;

    assert!(
        matches!(report.stopped, StopCause::Done),
        "{:?}",
        report.stopped
    );
    assert_eq!(report.rounds, 2);
    assert!((report.cost_usd - 0.30).abs() < 1e-9);
    assert_eq!(report.input_tokens, 200);
    assert_eq!(host.calls.len(), 4, "{:?}", host.calls);
    assert!(host.submitted);

    // The second request carries the echo: the text block and the two answered
    // tool-use blocks, then a user turn with exactly two results.
    let second = mock.calls().get(1).cloned().expect("two calls");
    assert_eq!(second.messages.len(), 3);
    let echo = &second.messages[1];
    assert_eq!(echo.role, Role::Assistant);
    let echoed_ids: Vec<&str> = echo
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(echoed_ids, vec!["r1", "r2"]);
    assert!(matches!(
        echo.content.first(),
        Some(ContentBlock::Text { .. })
    ));
    let results = &second.messages[2];
    assert_eq!(results.role, Role::User);
    assert_eq!(results.content.len(), 2);
    assert!(results.content.iter().all(|b| matches!(
        b,
        ContentBlock::ToolResult {
            is_error: false,
            ..
        }
    )));
    assert_eq!(second.messages[0], ChatMessage::user("Plan a park."));
}

#[tokio::test]
async fn a_text_reply_gets_one_repair_then_the_run_is_over() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let svc = service(&mock);
    mock.push_response(text("I would rather chat."));
    mock.push_response(text("Still chatting."));
    let mut host = Host {
        repair: Some("Call a tool.".into()),
        ..Host::default()
    };
    let report = run(&svc, spec(6, None), &mut host).await;
    assert!(
        matches!(report.stopped, StopCause::Done),
        "{:?}",
        report.stopped
    );
    assert_eq!(report.rounds, 2);
    assert_eq!(host.texts.len(), 2);
    // The repair rode as an assistant turn plus the prompt.
    let second = mock.calls().get(1).cloned().expect("two calls");
    assert_eq!(second.messages.len(), 3);
    assert_eq!(
        second.messages[1],
        ChatMessage::assistant("I would rather chat.")
    );
    assert_eq!(second.messages[2], ChatMessage::user("Call a tool."));

    // A second repair is not offered: the run ends with nothing usable.
    let mock = Arc::new(MockProvider::new("openrouter"));
    let svc = service(&mock);
    mock.push_response(text("one"));
    mock.push_response(text("two"));
    struct AlwaysRepair;
    impl ToolHost for AlwaysRepair {
        fn call(&mut self, _: &str, _: &serde_json::Value) -> Option<ToolReply> {
            None
        }
        fn on_text(&mut self, _: &CompletionResponse) -> TextVerdict {
            TextVerdict::Repair("again".into())
        }
    }
    let report = run(&svc, spec(6, None), &mut AlwaysRepair).await;
    assert!(
        matches!(report.stopped, StopCause::NoTool),
        "{:?}",
        report.stopped
    );
    assert_eq!(report.rounds, 2);
    assert_eq!(report.last_text, "two");
}

#[tokio::test]
async fn the_round_bound_stops_a_run_that_keeps_reading() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let svc = service(&mock);
    mock.set_default(Ok(reply(
        vec![tool_use("r", "read", serde_json::json!({}))],
        Some(0.01),
    )));
    let mut host = Host::default();
    let report = run(&svc, spec(3, None), &mut host).await;
    assert!(
        matches!(report.stopped, StopCause::Done),
        "{:?}",
        report.stopped
    );
    assert_eq!(report.rounds, 3, "three calls and no fourth");
    assert_eq!(mock.call_count(), 3);
    // The last round's read was answered but could not continue: the run is
    // done at the bound rather than left mid-conversation.
    assert_eq!(host.calls.len(), 3);
}

#[tokio::test]
async fn the_cost_cap_is_checked_before_a_call_and_stops_the_run() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let svc = service(&mock);
    mock.set_default(Ok(reply(
        vec![tool_use("r", "read", serde_json::json!({}))],
        Some(0.60),
    )));
    let mut host = Host::default();
    let report = run(&svc, spec(10, Some(1.0)), &mut host).await;
    assert!(
        matches!(report.stopped, StopCause::Cap),
        "{:?}",
        report.stopped
    );
    // 0.60 after one call is under the cap, so a second is made; 1.20 is not.
    assert_eq!(report.rounds, 2);
    assert_eq!(mock.call_count(), 2);
    assert!((report.cost_usd - 1.20).abs() < 1e-9);

    // A provider that prices nothing never trips the cap.
    let mock = Arc::new(MockProvider::new("openrouter"));
    let svc = service(&mock);
    mock.set_default(Ok(reply(
        vec![tool_use("r", "read", serde_json::json!({}))],
        None,
    )));
    let report = run(&svc, spec(3, Some(0.01)), &mut Host::default()).await;
    assert!(
        matches!(report.stopped, StopCause::Done),
        "{:?}",
        report.stopped
    );
    assert_eq!(report.rounds, 3);
}

#[tokio::test]
async fn a_finished_host_stops_the_run_before_the_next_call() {
    let mock = Arc::new(MockProvider::new("openrouter"));
    let svc = service(&mock);
    mock.set_default(Ok(reply(
        vec![tool_use("r", "read", serde_json::json!({}))],
        None,
    )));
    struct Done;
    impl ToolHost for Done {
        fn call(&mut self, _: &str, _: &serde_json::Value) -> Option<ToolReply> {
            None
        }
        fn on_text(&mut self, _: &CompletionResponse) -> TextVerdict {
            TextVerdict::Done
        }
        fn finished(&self) -> bool {
            true
        }
    }
    let report = run(&svc, spec(3, None), &mut Done).await;
    assert!(matches!(report.stopped, StopCause::Done));
    assert_eq!(report.rounds, 0);
    assert_eq!(mock.call_count(), 0);

    // A round whose every call is declined ends the run: nothing invited more.
    let mut host = Host::default();
    mock.set_default(Ok(reply(
        vec![tool_use("s", "skip", serde_json::json!({}))],
        None,
    )));
    let report = run(&svc, spec(3, None), &mut host).await;
    assert!(matches!(report.stopped, StopCause::Done));
    assert_eq!(report.rounds, 1);
    assert_eq!(host.calls, vec!["skip:{}".to_string()]);
}

#[tokio::test]
async fn a_service_refusal_ends_the_run_with_the_error_and_what_it_had() {
    // The daily feature budget trips on the second call: the report carries the
    // error and the host keeps the first round's answer.
    let mock = Arc::new(MockProvider::new("openrouter"));
    let mut budgets = BudgetConfig::default();
    budgets.per_feature.insert(
        "worldgen_plan".into(),
        BudgetLimit {
            tokens: None,
            cost_usd: Some(0.05),
        },
    );
    let svc = LlmService::builder()
        .provider("openrouter", Arc::clone(&mock) as Arc<dyn LlmProvider>)
        .route("worldgen_plan", route())
        .budgets(budgets)
        .global_enabled(true)
        .build();
    mock.set_default(Ok(reply(
        vec![tool_use("r", "read", serde_json::json!({}))],
        Some(0.10),
    )));
    let mut host = Host::default();
    let report = run(&svc, spec(4, None), &mut host).await;
    assert!(
        matches!(
            report.stopped,
            StopCause::Error(LlmError::BudgetExceeded { .. })
        ),
        "{:?}",
        report.stopped
    );
    assert_eq!(report.rounds, 1);
    assert_eq!(host.calls.len(), 1);

    // The kill switch refuses the first call outright.
    svc.kill_switch().set_global(true);
    let report = run(&svc, spec(4, None), &mut Host::default()).await;
    assert!(matches!(
        report.stopped,
        StopCause::Error(LlmError::Disabled)
    ));
    assert_eq!(report.rounds, 0);
}
