//! Adapter WIRE tests: a local mock HTTP server (wiremock) asserts the
//! OUTBOUND request shape and parses canned responses. NO real network, NO real
//! keys. Covers the validated OpenRouter + Anthropic facts.

#![cfg(all(feature = "anthropic", feature = "openai"))]

use std::collections::HashMap;
use std::time::Duration;

use futures::StreamExt;
use mush_llm::anthropic::AnthropicProvider;
use mush_llm::config::{LlmConfig, ProviderConfig, ProviderKind, RouteConfig};
use mush_llm::openai::OpenAiProvider;
use mush_llm::provider::{LlmError, LlmProvider};
use mush_llm::service::{LlmFeature, LlmService};
use mush_llm::types::{
    CompletionRequest, CompletionResponse, ContentBlock, Effort, ReasoningConfig, Role, Sampling,
    StopReason, StreamEvent, ToolChoice, ToolDef,
};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TIMEOUT: Duration = Duration::from_secs(10);

fn openai_response_of_size(total_bytes: usize) -> (Vec<u8>, usize) {
    const PREFIX: &str = r#"{"model":"m","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":""#;
    const SUFFIX: &str = r#""}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
    assert!(total_bytes >= PREFIX.len() + SUFFIX.len());
    let text_bytes = total_bytes - PREFIX.len() - SUFFIX.len();
    let mut body = Vec::with_capacity(total_bytes);
    body.extend_from_slice(PREFIX.as_bytes());
    body.extend(std::iter::repeat_n(b'x', text_bytes));
    body.extend_from_slice(SUFFIX.as_bytes());
    assert_eq!(body.len(), total_bytes);
    (body, text_bytes)
}

fn provider_cfg(kind: ProviderKind, base_url: String) -> ProviderConfig {
    ProviderConfig {
        kind,
        base_url,
        api_key_env: None,
        requests_per_minute: 60,
        referer: None,
        title: None,
        anthropic_version: None,
        price_input_per_mtok: None,
        price_output_per_mtok: None,
        tool_calling: true,
        forced_tool_choice: true,
        native_structured_output: false,
    }
}

async fn received_body(server: &MockServer) -> Value {
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1, "exactly one request expected");
    serde_json::from_slice(&reqs[0].body).expect("request body is json")
}

async fn received_header(server: &MockServer, name: &str) -> Option<String> {
    let reqs = server.received_requests().await.unwrap();
    reqs[0]
        .headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

// -- Anthropic -------------------------------------------------------------

#[tokio::test]
async fn anthropic_request_shape() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_1",
            "model": "claude-opus-5",
            "role": "assistant",
            "content": [{"type": "text", "text": "hi"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 3}
        })))
        .mount(&server)
        .await;

    let mut cfg = provider_cfg(ProviderKind::Anthropic, server.uri());
    cfg.price_input_per_mtok = Some(5.0);
    cfg.price_output_per_mtok = Some(25.0);
    let provider = AnthropicProvider::new("anthropic", &cfg, "test-key".to_string()).unwrap();

    // Flagship model + a temperature that must be dropped, a cached system
    // prefix, forced tool choice, and high reasoning effort.
    let req = CompletionRequest {
        model: "claude-opus-5".to_string(),
        system: Some("You are the worldgen engine.".to_string()),
        messages: vec![mush_llm::types::ChatMessage::user("Describe a pawn shop.")],
        tools: vec![ToolDef {
            name: "submit_room".to_string(),
            description: "submit the room".to_string(),
            input_schema: json!({"type": "object", "additionalProperties": false}),
            strict: true,
        }],
        tool_choice: ToolChoice::Tool("submit_room".to_string()),
        max_tokens: 2048,
        sampling: Sampling {
            temperature: Some(0.7),
            top_p: None,
        },
        stop_sequences: vec![],
        reasoning: Some(ReasoningConfig::Effort(Effort::High)),
    };

    let resp = provider.complete(&req, TIMEOUT).await.unwrap();
    assert_eq!(resp.usage.input_tokens, 10);
    assert_eq!(resp.usage.cache_read_tokens, 3);
    // No cost field on the wire → estimated from configured price.
    let expected = 5.0 * 10.0 / 1e6 + 25.0 * 5.0 / 1e6;
    assert!((resp.usage.cost_usd.unwrap() - expected).abs() < 1e-12);

    // Headers.
    assert_eq!(
        received_header(&server, "x-api-key").await.as_deref(),
        Some("test-key")
    );
    assert!(
        received_header(&server, "anthropic-version")
            .await
            .is_some()
    );

    // Body shape.
    let body = received_body(&server).await;
    assert_eq!(body["model"], "claude-opus-5");
    assert_eq!(body["max_tokens"], 2048);
    // Sampling dropped for the flagship generation.
    assert!(
        body.get("temperature").is_none(),
        "temperature must be dropped for opus-5"
    );
    // System is a cache-marked block.
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    // Reasoning via output_config effort.
    assert_eq!(body["output_config"]["effort"], "high");
    // Forced tool choice + strict tool + cached tool prefix.
    assert_eq!(body["tool_choice"]["type"], "tool");
    assert_eq!(body["tool_choice"]["name"], "submit_room");
    assert_eq!(body["tools"][0]["strict"], true);
    assert_eq!(body["tools"][0]["cache_control"]["type"], "ephemeral");
}

#[tokio::test]
async fn anthropic_refusal_maps_to_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_2",
            "model": "claude-opus-5",
            "role": "assistant",
            "content": [],
            "stop_reason": "refusal",
            "usage": {"input_tokens": 8, "output_tokens": 0}
        })))
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Anthropic, server.uri());
    let provider = AnthropicProvider::new("anthropic", &cfg, "k".to_string()).unwrap();
    let req = CompletionRequest::simple("do something disallowed");
    let err = provider.complete(&req, TIMEOUT).await.unwrap_err();
    assert!(matches!(err, LlmError::Refused { .. }));
}

#[tokio::test]
async fn anthropic_non_flagship_keeps_temperature() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "claude-haiku-4-5",
            "content": [{"type": "text", "text": "ok"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })))
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Anthropic, server.uri());
    let provider = AnthropicProvider::new("anthropic", &cfg, "k".to_string()).unwrap();
    let mut req = CompletionRequest::simple("hi");
    req.model = "claude-haiku-4-5".to_string();
    req.sampling.temperature = Some(0.4);
    provider.complete(&req, TIMEOUT).await.unwrap();

    let body = received_body(&server).await;
    let temp = body["temperature"].as_f64().expect("temperature sent");
    assert!(
        (temp - 0.4).abs() < 1e-6,
        "non-flagship keeps sampling (got {temp})"
    );
}

#[tokio::test]
async fn anthropic_auth_error_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "bad key"})))
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Anthropic, server.uri());
    let provider = AnthropicProvider::new("anthropic", &cfg, "k".to_string()).unwrap();
    let err = provider
        .complete(&CompletionRequest::simple("hi"), TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Auth));
}

// -- OpenRouter (OpenAI-compatible) ---------------------------------------

#[tokio::test]
async fn openrouter_request_shape_and_usage_parsing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "gen-1",
            "model": "z-ai/glm-5.3-flash",
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {"role": "assistant", "content": "pong"}
            }],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 7,
                "cost": 6.4e-6,
                "completion_tokens_details": {"reasoning_tokens": 4},
                "prompt_tokens_details": {"cached_tokens": 2}
            }
        })))
        .mount(&server)
        .await;

    let mut cfg = provider_cfg(ProviderKind::Openai, server.uri());
    cfg.referer = Some("https://github.com/rustmush".to_string());
    cfg.title = Some("rustmush".to_string());
    let provider = OpenAiProvider::new("openrouter", &cfg, Some("or-key".to_string())).unwrap();

    let req = CompletionRequest {
        model: "z-ai/glm-5.3-flash".to_string(),
        system: Some("system prefix".to_string()),
        messages: vec![mush_llm::types::ChatMessage::user("ping")],
        tools: vec![],
        tool_choice: ToolChoice::Auto,
        max_tokens: 200,
        sampling: Sampling::default(),
        stop_sequences: vec![],
        reasoning: Some(ReasoningConfig::Effort(Effort::Low)),
    };

    let resp = provider.complete(&req, TIMEOUT).await.unwrap();
    assert_eq!(resp.text(), "pong");
    assert_eq!(resp.stop_reason, StopReason::EndTurn);
    // usage.cost → cost_usd (real money); reasoning_tokens surfaced.
    assert_eq!(resp.usage.cost_usd, Some(6.4e-6));
    assert_eq!(resp.usage.reasoning_tokens, 4);
    assert_eq!(resp.usage.cache_read_tokens, 2);
    assert_eq!(resp.usage.input_tokens, 12);

    // Attribution headers + Bearer key.
    assert_eq!(
        received_header(&server, "http-referer").await.as_deref(),
        Some("https://github.com/rustmush")
    );
    assert_eq!(
        received_header(&server, "x-title").await.as_deref(),
        Some("rustmush")
    );
    assert_eq!(
        received_header(&server, "authorization").await.as_deref(),
        Some("Bearer or-key")
    );

    // Body: model verbatim, reasoning field present, system down-converted.
    let body = received_body(&server).await;
    assert_eq!(body["model"], "z-ai/glm-5.3-flash");
    assert_eq!(body["reasoning"]["effort"], "low");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][0]["content"], "system prefix");
    assert_eq!(body["messages"][1]["role"], "user");
    // Opts into OpenRouter Usage Accounting so `usage.cost` (real money) is
    // returned — without this the cost budget would settle at $0.
    assert_eq!(
        body["usage"]["include"], true,
        "must request usage accounting: {body}"
    );
}

#[tokio::test]
async fn openrouter_reasoning_disabled_maps_to_enabled_false() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "deepseek/deepseek-v4-flash",
            "choices": [{"index": 0, "finish_reason": "stop",
                         "message": {"role": "assistant", "content": "ok"}}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2}
        })))
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, None).unwrap();
    let mut req = CompletionRequest::simple("hi");
    req.model = "deepseek/deepseek-v4-flash".to_string();
    req.reasoning = Some(ReasoningConfig::Disabled);
    provider.complete(&req, TIMEOUT).await.unwrap();

    let body = received_body(&server).await;
    assert_eq!(body["reasoning"]["enabled"], false);
    // Keyless deployment sends no Authorization header.
    assert!(received_header(&server, "authorization").await.is_none());
}

#[tokio::test]
async fn openrouter_tool_call_parsed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "z-ai/glm-5.3",
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "submit_room", "arguments": "{\"desc\":\"A dim room.\"}"}
                    }]
                }
            }],
            "usage": {"prompt_tokens": 20, "completion_tokens": 15}
        })))
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, Some("k".to_string())).unwrap();
    let req = CompletionRequest {
        model: "z-ai/glm-5.3".to_string(),
        system: None,
        messages: vec![mush_llm::types::ChatMessage::user("gen")],
        tools: vec![ToolDef {
            name: "submit_room".to_string(),
            description: "d".to_string(),
            input_schema: json!({"type": "object"}),
            strict: true,
        }],
        tool_choice: ToolChoice::Tool("submit_room".to_string()),
        max_tokens: 512,
        sampling: Sampling::default(),
        stop_sequences: vec![],
        reasoning: None,
    };

    let resp = provider.complete(&req, TIMEOUT).await.unwrap();
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    let (name, input) = resp.first_tool_use().unwrap();
    assert_eq!(name, "submit_room");
    assert_eq!(input["desc"], "A dim room.");

    // Outbound: tool arguments are a JSON string; tool_choice forces the tool.
    let body = received_body(&server).await;
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "submit_room");
    assert_eq!(body["tool_choice"]["function"]["name"], "submit_room");
}

#[tokio::test]
async fn openrouter_malformed_tool_args_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "z-ai/glm-5.3",
            "choices": [{
                "index": 0,
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "c",
                        "type": "function",
                        "function": {"name": "submit_room", "arguments": "{not valid json"}
                    }]
                }
            }],
            "usage": {"prompt_tokens": 5, "completion_tokens": 3}
        })))
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, Some("k".to_string())).unwrap();
    let err = provider
        .complete(&CompletionRequest::simple("gen"), TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::MalformedOutput(_)));
}

#[tokio::test]
async fn openai_down_conversion_splits_tool_results() {
    // A ToolResult in a user turn becomes a role:"tool" message; assistant
    // ToolUse becomes tool_calls with string arguments.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "m",
            "choices": [{"index": 0, "finish_reason": "stop",
                         "message": {"role": "assistant", "content": "done"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        })))
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, None).unwrap();

    let req = CompletionRequest {
        model: "m".to_string(),
        system: None,
        messages: vec![
            mush_llm::types::ChatMessage::user("look at the sign"),
            mush_llm::types::ChatMessage {
                role: Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: "t1".to_string(),
                    name: "look_at".to_string(),
                    input: json!({"target": "sign"}),
                }],
            },
            mush_llm::types::ChatMessage {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "t1".to_string(),
                    content: "The sign reads CLOSED.".to_string(),
                    is_error: false,
                }],
            },
        ],
        tools: vec![],
        tool_choice: ToolChoice::Auto,
        max_tokens: 64,
        sampling: Sampling::default(),
        stop_sequences: vec![],
        reasoning: None,
    };
    provider.complete(&req, TIMEOUT).await.unwrap();

    let body = received_body(&server).await;
    let msgs = body["messages"].as_array().unwrap();
    // user, assistant(tool_calls), tool
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(
        msgs[1]["tool_calls"][0]["function"]["arguments"],
        "{\"target\":\"sign\"}"
    );
    assert_eq!(msgs[2]["role"], "tool");
    assert_eq!(msgs[2]["tool_call_id"], "t1");
}

#[tokio::test]
async fn openrouter_content_filter_maps_to_refused() {
    // A content-filter stop must surface as a typed refusal (symmetric with the
    // Anthropic adapter), not an Ok with empty content the caller mis-parses.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "z-ai/glm-5.3",
            "choices": [{
                "index": 0,
                "finish_reason": "content_filter",
                "message": {"role": "assistant", "content": ""}
            }],
            "usage": {"prompt_tokens": 9, "completion_tokens": 0, "cost": 1.0e-6}
        })))
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, Some("k".to_string())).unwrap();
    let err = provider
        .complete(
            &CompletionRequest::simple("do something disallowed"),
            TIMEOUT,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::Refused { .. }), "got {err:?}");
}

// -- Secret hygiene --------------------------------------------------------

#[tokio::test]
async fn error_body_echoing_a_key_is_scrubbed() {
    // A provider error body that echoes an API key inside compact JSON (no
    // surrounding whitespace) must never reach the error string verbatim.
    let leaked = "sk-orV1ABCdef0123456789ghijklmnopqrstuvwx";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(format!("{{\"error\":{{\"apikey\":\"{leaked}\"}}}}")),
        )
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, Some("k".to_string())).unwrap();
    let err = provider
        .complete(&CompletionRequest::simple("hi"), TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::InvalidRequest(_)));
    let msg = err.to_string();
    assert!(!msg.contains(leaked), "key leaked into error string: {msg}");
    assert!(msg.contains("[redacted]"), "expected redaction: {msg}");
}

#[test]
fn provider_debug_never_contains_the_key() {
    let secret = "sk-super-secret-key-value-01234567890abcdef";
    let ocfg = provider_cfg(
        ProviderKind::Openai,
        "https://openrouter.ai/api/v1".to_string(),
    );
    let op = OpenAiProvider::new("openrouter", &ocfg, Some(secret.to_string())).unwrap();
    assert!(
        !format!("{op:?}").contains(secret),
        "OpenAiProvider Debug leaked the key"
    );
    let acfg = provider_cfg(
        ProviderKind::Anthropic,
        "https://api.anthropic.com".to_string(),
    );
    let ap = AnthropicProvider::new("anthropic", &acfg, secret.to_string()).unwrap();
    assert!(
        !format!("{ap:?}").contains(secret),
        "AnthropicProvider Debug leaked the key"
    );
}

// -- Provider response body boundary --------------------------------------

#[tokio::test]
async fn openai_declared_oversize_response_is_rejected() {
    let server = MockServer::start().await;
    let (body, _) = openai_response_of_size(mush_llm::MAX_PROVIDER_RESPONSE_BYTES + 1);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                // wiremock/hyper derives the exact Content-Length from this
                // fixed body, exercising the header preflight.
                .set_body_raw(body, "application/json"),
        )
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, None).unwrap();
    let err = provider
        .complete(&CompletionRequest::simple("hi"), TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::MalformedOutput(_)), "got {err:?}");
    assert!(err.to_string().contains("response exceeds"));
}

#[tokio::test]
async fn anthropic_chunked_oversize_response_is_rejected() {
    let server = MockServer::start().await;
    let (body, _) = openai_response_of_size(mush_llm::MAX_PROVIDER_RESPONSE_BYTES + 1);
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(body, "application/json")
                // Force HTTP/1.1 transfer coding so there is no declared
                // length to preflight; the incremental counter must stop it.
                .insert_header("transfer-encoding", "chunked"),
        )
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Anthropic, server.uri());
    let provider = AnthropicProvider::new("anthropic", &cfg, "k".to_string()).unwrap();
    let err = provider
        .complete(&CompletionRequest::simple("hi"), TIMEOUT)
        .await
        .unwrap_err();
    assert!(matches!(err, LlmError::MalformedOutput(_)), "got {err:?}");
    assert!(err.to_string().contains("response exceeds"));
}

#[tokio::test]
async fn openai_response_at_exact_limit_succeeds() {
    let server = MockServer::start().await;
    let (body, expected_text_bytes) =
        openai_response_of_size(mush_llm::MAX_PROVIDER_RESPONSE_BYTES);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, None).unwrap();
    let response = provider
        .complete(&CompletionRequest::simple("hi"), TIMEOUT)
        .await
        .unwrap();
    assert_eq!(response.text().len(), expected_text_bytes);
}

#[tokio::test]
async fn openai_chunked_stream_stops_at_response_limit() {
    let server = MockServer::start().await;
    let sse = format!(
        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
        "x".repeat(mush_llm::MAX_PROVIDER_RESPONSE_BYTES)
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse, "text/event-stream")
                .insert_header("transfer-encoding", "chunked"),
        )
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, None).unwrap();
    let mut stream = provider
        .stream(&CompletionRequest::simple("hi"), TIMEOUT)
        .await
        .expect("chunked response has no declared oversize length");
    let err = loop {
        match stream.next().await {
            Some(Err(err)) => break err,
            Some(Ok(StreamEvent::Done(_))) => panic!("oversized stream emitted Done"),
            Some(Ok(_)) => {}
            None => panic!("oversized stream ended without an error"),
        }
    };
    assert!(matches!(err, LlmError::MalformedOutput(_)), "got {err:?}");
    assert!(err.to_string().contains("response exceeds"));
}

// -- from_config: keyless vs named-but-unset -------------------------------

fn worldgen_route() -> RouteConfig {
    RouteConfig {
        model: "z-ai/glm-5.3".to_string(),
        max_tokens: 100,
        timeout_s: 30,
        reasoning: None,
        reasoning_max_tokens: None,
    }
}

fn openrouter_provider(api_key_env: Option<&str>) -> ProviderConfig {
    let mut c = provider_cfg(
        ProviderKind::Openai,
        "https://openrouter.ai/api/v1".to_string(),
    );
    c.api_key_env = api_key_env.map(str::to_string);
    c
}

fn config_with(provider: ProviderConfig) -> LlmConfig {
    let mut providers = HashMap::new();
    providers.insert("openrouter".to_string(), provider);
    let mut routes = HashMap::new();
    routes.insert("worldgen_room".to_string(), worldgen_route());
    LlmConfig {
        enabled: true,
        providers,
        routes,
        ..Default::default()
    }
}

#[test]
fn named_but_unset_key_skips_provider_but_keyless_registers() {
    // A NAMED but unset key is a misconfiguration → skipped (route fails fast).
    let cfg = config_with(openrouter_provider(Some("RUSTMUSH_TEST_UNSET_KEY_9Z7Q")));
    let svc = LlmService::from_config(&cfg).expect("builds");
    assert!(
        svc.caps_for(LlmFeature::WorldgenRoom).is_none(),
        "provider with a named-but-unset key must be skipped"
    );

    // NO api_key_env is a legitimate keyless deployment (Ollama) → registered.
    let cfg = config_with(openrouter_provider(None));
    let svc = LlmService::from_config(&cfg).expect("builds");
    assert!(
        svc.caps_for(LlmFeature::WorldgenRoom).is_some(),
        "keyless (no api_key_env) provider must register"
    );
}

// -- Streaming assembly (real SSE through the adapters' parsers) ------------

async fn collect_stream(mut s: mush_llm::types::LlmStream) -> (String, Option<CompletionResponse>) {
    let mut text = String::new();
    let mut done = None;
    while let Some(ev) = s.next().await {
        match ev.expect("no mid-stream error") {
            StreamEvent::TextDelta(t) => text.push_str(&t),
            StreamEvent::Done(r) => done = Some(r),
            _ => {}
        }
    }
    (text, done)
}

#[tokio::test]
async fn openrouter_streaming_assembles_text_and_usage() {
    let sse = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hel\"}}]}\n\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"}}]}\n\n\
data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"cost\":0.0001}}\n\n\
data: [DONE]\n\n";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(sse.as_bytes().to_vec(), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let cfg = provider_cfg(ProviderKind::Openai, server.uri());
    let provider = OpenAiProvider::new("openrouter", &cfg, Some("k".to_string())).unwrap();
    let stream = provider
        .stream(&CompletionRequest::simple("ping"), TIMEOUT)
        .await
        .unwrap();
    let (text, done) = collect_stream(stream).await;
    assert_eq!(text, "Hello");
    let done = done.expect("Done event");
    assert_eq!(done.text(), "Hello");
    assert_eq!(done.usage.output_tokens, 2);
    assert_eq!(done.usage.input_tokens, 5);
    assert_eq!(done.usage.cost_usd, Some(0.0001));
    assert_eq!(done.stop_reason, StopReason::EndTurn);

    // Streaming opts into per-chunk usage.
    let body = received_body(&server).await;
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[tokio::test]
async fn anthropic_streaming_assembles_text_tool_and_usage() {
    let sse = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":2}}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"tu_1\",\"name\":\"submit_room\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"desc\\\":\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"A hall.\\\"}\"}}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":7}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(sse.as_bytes().to_vec(), "text/event-stream"),
        )
        .mount(&server)
        .await;

    let mut cfg = provider_cfg(ProviderKind::Anthropic, server.uri());
    cfg.price_input_per_mtok = Some(5.0);
    cfg.price_output_per_mtok = Some(25.0);
    let provider = AnthropicProvider::new("anthropic", &cfg, "k".to_string()).unwrap();
    let stream = provider
        .stream(&CompletionRequest::simple("gen"), TIMEOUT)
        .await
        .unwrap();
    let (text, done) = collect_stream(stream).await;
    assert_eq!(text, "Hi");
    let done = done.expect("Done event");
    assert_eq!(done.usage.input_tokens, 10);
    assert_eq!(done.usage.output_tokens, 7);
    assert_eq!(done.usage.cache_read_tokens, 2);
    // Anthropic has no wire cost → estimated from configured per-MTok price.
    let expected = 5.0 * 10.0 / 1e6 + 25.0 * 7.0 / 1e6;
    assert!((done.usage.cost_usd.unwrap() - expected).abs() < 1e-12);
    // The streamed tool_use block assembled into a parsed input.
    let (name, input) = done.first_tool_use().expect("tool use assembled");
    assert_eq!(name, "submit_room");
    assert_eq!(input["desc"], "A hall.");
    assert_eq!(done.stop_reason, StopReason::ToolUse);
}
