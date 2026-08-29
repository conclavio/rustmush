//! OpenAI-compatible chat-completions adapter (feature `openai`).
//!
//! One client covering OpenRouter (primary) and generic OpenAI/Ollama/vLLM,
//! differing only by `base_url`, key, headers, and caps. Per the validated
//! OpenRouter facts: `Authorization: Bearer` + `HTTP-Referer`/`X-Title`
//! headers, the `reasoning` request field mapped from [`ReasoningConfig`], and
//! response parsing that reads `usage.cost` (real money) into
//! [`Usage::cost_usd`] and `completion_tokens_details.reasoning_tokens` into
//! [`Usage::reasoning_tokens`]. Neutral types down-convert: `system` → a
//! leading `role:"system"` message, `ToolUse` → `assistant.tool_calls` (with
//! `arguments` as a JSON **string**), `ToolResult` → `role:"tool"` messages.

use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use serde_json::{Value, json};

use crate::config::ProviderConfig;
use crate::http;
use crate::provider::{LlmError, LlmProvider, ProviderCaps, ProviderId};
use crate::types::{
    CompletionRequest, CompletionResponse, ContentBlock, LlmStream, ReasoningConfig, Role,
    StopReason, StreamEvent, ToolChoice, Usage,
};

/// An OpenAI-compatible chat-completions client.
pub struct OpenAiProvider {
    id: ProviderId,
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    referer: Option<String>,
    title: Option<String>,
    caps: ProviderCaps,
}

impl std::fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "OpenAiProvider(id={}, base_url={})",
            self.id, self.base_url
        )
    }
}

impl OpenAiProvider {
    /// Build from resolved config. `api_key` is already resolved from its env
    /// var (or `None` for keyless deployments like local Ollama).
    pub fn new(
        id: impl Into<ProviderId>,
        cfg: &ProviderConfig,
        api_key: Option<String>,
    ) -> Result<OpenAiProvider, LlmError> {
        Ok(OpenAiProvider {
            id: id.into(),
            client: http::client()?,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            api_key,
            referer: cfg.referer.clone(),
            title: cfg.title.clone(),
            caps: ProviderCaps {
                tool_calling: cfg.tool_calling,
                forced_tool_choice: cfg.forced_tool_choice,
                native_structured_output: cfg.native_structured_output,
                sampling_params: true,
                streaming_usage: true,
                reasoning: true,
            },
        })
    }

    fn url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn body(&self, req: &CompletionRequest, stream: bool) -> Value {
        let mut body = serde_json::Map::new();
        body.insert("model".into(), json!(req.model));
        body.insert("max_tokens".into(), json!(req.max_tokens));
        body.insert("messages".into(), Value::Array(messages_json(req)));

        // Opt into OpenRouter Usage Accounting: without this the response
        // `usage` object omits the real-money `cost` field, and the budget
        // ledger would settle every OpenRouter call at cost 0 — the exact
        // silent-overspend the cost caps exist to prevent. Harmless to generic
        // OpenAI-compatible servers, which ignore the extra field.
        body.insert("usage".into(), json!({"include": true}));

        if !req.tools.is_empty() {
            let tools: Vec<Value> = req
                .tools
                .iter()
                .map(|t| {
                    let mut func = serde_json::Map::new();
                    func.insert("name".into(), json!(t.name));
                    func.insert("description".into(), json!(t.description));
                    func.insert("parameters".into(), t.input_schema.clone());
                    if t.strict {
                        func.insert("strict".into(), json!(true));
                    }
                    json!({"type": "function", "function": Value::Object(func)})
                })
                .collect();
            body.insert("tools".into(), Value::Array(tools));
            if let Some(tc) = tool_choice_json(&req.tool_choice) {
                body.insert("tool_choice".into(), tc);
            }
        }

        // Reasoning field (OpenRouter): keep max_tokens headroom for it.
        match req.reasoning {
            Some(ReasoningConfig::Effort(e)) => {
                body.insert("reasoning".into(), json!({"effort": e.as_str()}));
            }
            Some(ReasoningConfig::Disabled) => {
                body.insert("reasoning".into(), json!({"enabled": false}));
            }
            Some(ReasoningConfig::MaxTokens(n)) => {
                body.insert("reasoning".into(), json!({"max_tokens": n}));
            }
            None => {}
        }

        if self.caps.sampling_params {
            if let Some(t) = req.sampling.temperature {
                body.insert("temperature".into(), json!(t));
            }
            if let Some(p) = req.sampling.top_p {
                body.insert("top_p".into(), json!(p));
            }
        }

        if !req.stop_sequences.is_empty() {
            body.insert("stop".into(), json!(req.stop_sequences));
        }

        if stream {
            body.insert("stream".into(), json!(true));
            body.insert("stream_options".into(), json!({"include_usage": true}));
        }

        Value::Object(body)
    }

    fn request(&self, body: &Value, timeout: Duration) -> reqwest::RequestBuilder {
        let mut rb = self
            .client
            .post(self.url())
            .header("content-type", "application/json")
            .timeout(timeout);
        if let Some(key) = &self.api_key {
            rb = rb.header("authorization", format!("Bearer {key}"));
        }
        if let Some(r) = &self.referer {
            rb = rb.header("HTTP-Referer", r);
        }
        if let Some(t) = &self.title {
            rb = rb.header("X-Title", t);
        }
        rb.json(body)
    }
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn caps(&self) -> ProviderCaps {
        self.caps
    }

    async fn complete(
        &self,
        req: &CompletionRequest,
        timeout: Duration,
    ) -> Result<CompletionResponse, LlmError> {
        let body = self.body(req, false);
        let resp = self
            .request(&body, timeout)
            .send()
            .await
            .map_err(|e| http::transport_error(&e))?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let text = resp.text().await.map_err(|e| http::transport_error(&e))?;
        if !status.is_success() {
            return Err(http::status_error(status, &headers, &text));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| LlmError::MalformedOutput(format!("response json: {e}")))?;
        parse_response(&v)
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
        timeout: Duration,
    ) -> Result<LlmStream, LlmError> {
        use eventsource_stream::Eventsource;

        let body = self.body(req, true);
        let resp = self
            .request(&body, timeout)
            .send()
            .await
            .map_err(|e| http::transport_error(&e))?;
        let status = resp.status();
        if !status.is_success() {
            let headers = resp.headers().clone();
            let text = resp.text().await.unwrap_or_default();
            return Err(http::status_error(status, &headers, &text));
        }

        let model = req.model.clone();
        let (tx, rx) = mpsc::unbounded::<Result<StreamEvent, LlmError>>();
        let mut events = resp.bytes_stream().eventsource();

        tokio::spawn(async move {
            let mut asm = StreamAsm::new(model);
            while let Some(ev) = events.next().await {
                match ev {
                    Ok(event) => {
                        if event.data.trim() == "[DONE]" {
                            break;
                        }
                        asm.on_data(&event.data, &tx);
                    }
                    Err(e) => {
                        let _ = tx.unbounded_send(Err(LlmError::Network(e.to_string())));
                        return;
                    }
                }
            }
            let _ = tx.unbounded_send(Ok(StreamEvent::Done(asm.finish())));
        });

        Ok(rx.boxed())
    }
}

// -- streaming assembly ----------------------------------------------------

struct ToolAsm {
    id: String,
    name: String,
    args: String,
    started: bool,
}

struct StreamAsm {
    model: String,
    text: String,
    tools: Vec<ToolAsm>,
    stop: StopReason,
    usage: Usage,
}

impl StreamAsm {
    fn new(model: String) -> StreamAsm {
        StreamAsm {
            model,
            text: String::new(),
            tools: Vec::new(),
            stop: StopReason::EndTurn,
            usage: Usage::default(),
        }
    }

    fn on_data(&mut self, data: &str, tx: &mpsc::UnboundedSender<Result<StreamEvent, LlmError>>) {
        let v: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return,
        };
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            self.usage = parse_usage(u);
        }
        if let Some(m) = v.get("model").and_then(Value::as_str)
            && !m.is_empty()
        {
            self.model = m.to_string();
        }
        let Some(choice) = v
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        else {
            return;
        };
        if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
            self.stop = parse_finish(fr);
        }
        let Some(delta) = choice.get("delta") else {
            return;
        };
        if let Some(t) = delta.get("content").and_then(Value::as_str)
            && !t.is_empty()
        {
            self.text.push_str(t);
            let _ = tx.unbounded_send(Ok(StreamEvent::TextDelta(t.to_string())));
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let idx = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                while self.tools.len() <= idx {
                    self.tools.push(ToolAsm {
                        id: String::new(),
                        name: String::new(),
                        args: String::new(),
                        started: false,
                    });
                }
                let slot = &mut self.tools[idx];
                if let Some(id) = call.get("id").and_then(Value::as_str)
                    && !id.is_empty()
                {
                    slot.id = id.to_string();
                }
                if let Some(func) = call.get("function") {
                    if let Some(name) = func.get("name").and_then(Value::as_str)
                        && !name.is_empty()
                    {
                        slot.name = name.to_string();
                    }
                    if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                        slot.args.push_str(args);
                        let _ = tx.unbounded_send(Ok(StreamEvent::ToolInputDelta {
                            id: slot.id.clone(),
                            json_fragment: args.to_string(),
                        }));
                    }
                }
                if !slot.started && !slot.name.is_empty() {
                    slot.started = true;
                    let _ = tx.unbounded_send(Ok(StreamEvent::ToolUseStart {
                        id: slot.id.clone(),
                        name: slot.name.clone(),
                    }));
                }
            }
        }
    }

    fn finish(self) -> CompletionResponse {
        let mut content = Vec::new();
        if !self.text.is_empty() {
            content.push(ContentBlock::Text { text: self.text });
        }
        for t in self.tools {
            content.push(ContentBlock::ToolUse {
                id: t.id,
                name: t.name,
                input: serde_json::from_str(&t.args).unwrap_or(Value::Object(Default::default())),
            });
        }
        CompletionResponse {
            content,
            stop_reason: self.stop,
            usage: self.usage,
            model: self.model,
        }
    }
}

// -- mapping helpers -------------------------------------------------------

/// Down-convert neutral messages: system → leading `role:"system"`; each
/// `ToolResult` → its own `role:"tool"` message; `ToolUse` → assistant
/// `tool_calls` with string `arguments`.
fn messages_json(req: &CompletionRequest) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(sys) = &req.system {
        out.push(json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        match m.role {
            Role::User => {
                let mut text = String::new();
                for b in &m.content {
                    match b {
                        ContentBlock::Text { text: t } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            out.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            }));
                        }
                        ContentBlock::ToolUse { .. } => {}
                    }
                }
                if !text.is_empty() {
                    out.push(json!({"role": "user", "content": text}));
                }
            }
            Role::Assistant => {
                let mut text = String::new();
                let mut tool_calls = Vec::new();
                for b in &m.content {
                    match b {
                        ContentBlock::Text { text: t } => text.push_str(t),
                        ContentBlock::ToolUse { id, name, input } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": serde_json::to_string(input)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                },
                            }));
                        }
                        ContentBlock::ToolResult { .. } => {}
                    }
                }
                let mut msg = serde_json::Map::new();
                msg.insert("role".into(), json!("assistant"));
                if tool_calls.is_empty() {
                    msg.insert("content".into(), json!(text));
                } else {
                    msg.insert(
                        "content".into(),
                        if text.is_empty() {
                            Value::Null
                        } else {
                            json!(text)
                        },
                    );
                    msg.insert("tool_calls".into(), Value::Array(tool_calls));
                }
                out.push(Value::Object(msg));
            }
        }
    }
    out
}

fn tool_choice_json(tc: &ToolChoice) -> Option<Value> {
    Some(match tc {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::Required => json!("required"),
        ToolChoice::None => json!("none"),
        ToolChoice::Tool(name) => json!({"type": "function", "function": {"name": name}}),
    })
}

fn parse_response(v: &Value) -> Result<CompletionResponse, LlmError> {
    let choice = v
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or_else(|| LlmError::MalformedOutput("no choices in response".to_string()))?;
    let msg = choice
        .get("message")
        .ok_or_else(|| LlmError::MalformedOutput("no message in choice".to_string()))?;

    let mut content = Vec::new();
    if let Some(t) = msg.get("content").and_then(Value::as_str)
        && !t.is_empty()
    {
        content.push(ContentBlock::Text {
            text: t.to_string(),
        });
    }
    if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let func = call.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let args_str = func
                .and_then(|f| f.get("arguments"))
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).map_err(|e| {
                LlmError::MalformedOutput(format!("tool arguments for {name:?}: {e}"))
            })?;
            content.push(ContentBlock::ToolUse {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                name,
                input,
            });
        }
    }

    let stop = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(parse_finish)
        .unwrap_or(StopReason::EndTurn);

    // A content-filter stop is the model declining. Surface it as a typed
    // refusal (symmetric with the Anthropic adapter), so callers hit the
    // refusal branch instead of parsing empty content as "malformed output"
    // and burning a second, also-refused, billable repair call.
    if stop == StopReason::Refusal {
        return Err(LlmError::Refused { category: None });
    }

    let usage = v
        .get("usage")
        .filter(|u| !u.is_null())
        .map(parse_usage)
        .unwrap_or_default();

    Ok(CompletionResponse {
        content,
        stop_reason: stop,
        usage,
        model: v
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

fn parse_usage(u: &Value) -> Usage {
    Usage {
        input_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: u
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read_tokens: u
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        reasoning_tokens: u
            .pointer("/completion_tokens_details/reasoning_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cost_usd: u.get("cost").and_then(Value::as_f64),
    }
}

fn parse_finish(s: &str) -> StopReason {
    match s {
        "stop" => StopReason::EndTurn,
        "length" => StopReason::MaxTokens,
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "content_filter" => StopReason::Refusal,
        other => StopReason::Other(other.to_string()),
    }
}
