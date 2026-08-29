//! Anthropic Messages API adapter (feature `anthropic`).
//!
//! Hand-rolled against `POST {base_url}/v1/messages` with `x-api-key` +
//! `anthropic-version` headers. Content-block mapping, tool_use/tool_result,
//! `cache_control: ephemeral` on the stable prefix (system + tools),
//! `stop_reason:"refusal"` → [`LlmError::Refused`], and sampling capability-
//! gated off for the opus-5/sonnet-5 flagship generation. No `cost` field on
//! the wire, so cost is estimated as tokens × configured per-MTok price.

use std::collections::BTreeMap;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use serde_json::{Value, json};

use crate::config::ProviderConfig;
use crate::http;
use crate::provider::{LlmError, LlmProvider, ProviderCaps, ProviderId};
use crate::types::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentBlock, Effort, LlmStream,
    ReasoningConfig, Role, StopReason, StreamEvent, ToolChoice, Usage,
};

const DEFAULT_VERSION: &str = "2023-06-01";

/// An Anthropic Messages API client.
pub struct AnthropicProvider {
    id: ProviderId,
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    version: String,
    price_input_per_mtok: Option<f64>,
    price_output_per_mtok: Option<f64>,
}

impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the key.
        write!(
            f,
            "AnthropicProvider(id={}, base_url={})",
            self.id, self.base_url
        )
    }
}

impl AnthropicProvider {
    /// Build from resolved config. `api_key` is already resolved from its env
    /// var by the caller (never inline, never logged).
    pub fn new(
        id: impl Into<ProviderId>,
        cfg: &ProviderConfig,
        api_key: String,
    ) -> Result<AnthropicProvider, LlmError> {
        Ok(AnthropicProvider {
            id: id.into(),
            client: http::client()?,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            api_key,
            version: cfg
                .anthropic_version
                .clone()
                .unwrap_or_else(|| DEFAULT_VERSION.to_string()),
            price_input_per_mtok: cfg.price_input_per_mtok,
            price_output_per_mtok: cfg.price_output_per_mtok,
        })
    }

    fn url(&self) -> String {
        format!("{}/v1/messages", self.base_url)
    }

    fn estimate_cost(&self, input: u64, output: u64) -> Option<f64> {
        match (self.price_input_per_mtok, self.price_output_per_mtok) {
            (None, None) => None,
            (pin, pout) => {
                let i = pin.unwrap_or(0.0) * (input as f64) / 1_000_000.0;
                let o = pout.unwrap_or(0.0) * (output as f64) / 1_000_000.0;
                Some(i + o)
            }
        }
    }

    fn body(&self, req: &CompletionRequest, stream: bool) -> Value {
        let mut body = serde_json::Map::new();
        body.insert("model".into(), json!(req.model));
        body.insert("max_tokens".into(), json!(req.max_tokens));
        if stream {
            body.insert("stream".into(), json!(true));
        }

        // System prompt as a cache-marked text block (the stable prefix).
        if let Some(sys) = &req.system {
            body.insert(
                "system".into(),
                json!([{
                    "type": "text",
                    "text": sys,
                    "cache_control": {"type": "ephemeral"},
                }]),
            );
        }

        body.insert(
            "messages".into(),
            Value::Array(req.messages.iter().map(message_json).collect()),
        );

        if !req.tools.is_empty() {
            let last = req.tools.len() - 1;
            let tools: Vec<Value> = req
                .tools
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut m = serde_json::Map::new();
                    m.insert("name".into(), json!(t.name));
                    m.insert("description".into(), json!(t.description));
                    m.insert("input_schema".into(), t.input_schema.clone());
                    if t.strict {
                        m.insert("strict".into(), json!(true));
                    }
                    // Extend the cached prefix over the tool defs.
                    if i == last {
                        m.insert("cache_control".into(), json!({"type": "ephemeral"}));
                    }
                    Value::Object(m)
                })
                .collect();
            body.insert("tools".into(), Value::Array(tools));
            if let Some(tc) = tool_choice_json(&req.tool_choice) {
                body.insert("tool_choice".into(), tc);
            }
        }

        // Sampling: dropped for the flagship generation (400 otherwise).
        if model_accepts_sampling(&req.model) {
            if let Some(t) = req.sampling.temperature {
                body.insert("temperature".into(), json!(t));
            }
            if let Some(p) = req.sampling.top_p {
                body.insert("top_p".into(), json!(p));
            }
        }

        // Reasoning: flagship uses output_config.effort; the older
        // thinking.budget_tokens form is for prior generations only, so a
        // `reasoning_max_tokens` route pointed at a flagship model is mapped to
        // an approximate effort rather than the raw budget the flagship rejects
        // with HTTP 400 (capability-gated the same way sampling is).
        match req.reasoning {
            Some(ReasoningConfig::Effort(e)) => {
                body.insert("output_config".into(), json!({"effort": effort_str(e)}));
            }
            Some(ReasoningConfig::MaxTokens(n)) => {
                if is_flagship_generation(&req.model) {
                    body.insert(
                        "output_config".into(),
                        json!({"effort": effort_str(effort_for_budget(n))}),
                    );
                } else {
                    body.insert(
                        "thinking".into(),
                        json!({"type": "enabled", "budget_tokens": n}),
                    );
                }
            }
            Some(ReasoningConfig::Disabled) | None => {}
        }

        if !req.stop_sequences.is_empty() {
            body.insert("stop_sequences".into(), json!(req.stop_sequences));
        }

        Value::Object(body)
    }

    fn request(&self, body: &Value, timeout: Duration) -> reqwest::RequestBuilder {
        self.client
            .post(self.url())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.version)
            .header("content-type", "application/json")
            .timeout(timeout)
            .json(body)
    }

    fn parse_response(&self, v: &Value) -> Result<CompletionResponse, LlmError> {
        let stop_raw = v.get("stop_reason").and_then(Value::as_str).unwrap_or("");
        if stop_raw == "refusal" {
            return Err(LlmError::Refused { category: None });
        }
        let content = v
            .get("content")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(parse_block).collect::<Vec<_>>())
            .unwrap_or_default();
        let u = v.get("usage");
        let input = u
            .and_then(|u| u.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = u
            .and_then(|u| u.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let cache_read = u
            .and_then(|u| u.get("cache_read_input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let usage = Usage {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            reasoning_tokens: 0,
            cost_usd: self.estimate_cost(input, output),
        };
        Ok(CompletionResponse {
            content,
            stop_reason: parse_stop(stop_raw),
            usage,
            model: v
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&req_model_fallback())
                .to_string(),
        })
    }
}

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn caps(&self) -> ProviderCaps {
        ProviderCaps {
            tool_calling: true,
            forced_tool_choice: true,
            native_structured_output: true,
            // Provider-level advisory; the adapter gates per model.
            sampling_params: false,
            streaming_usage: true,
            reasoning: true,
        }
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
        self.parse_response(&v)
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

        let price = (self.price_input_per_mtok, self.price_output_per_mtok);
        let model = req.model.clone();
        let (tx, rx) = mpsc::unbounded::<Result<StreamEvent, LlmError>>();
        let mut events = resp.bytes_stream().eventsource();

        tokio::spawn(async move {
            let mut asm = StreamAsm::new(model, price);
            while let Some(ev) = events.next().await {
                match ev {
                    Ok(event) => {
                        if !asm.on_event(&event, &tx) {
                            break;
                        }
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

enum BlockAsm {
    Text(String),
    Tool {
        id: String,
        name: String,
        json: String,
    },
}

struct StreamAsm {
    model: String,
    price: (Option<f64>, Option<f64>),
    blocks: BTreeMap<usize, BlockAsm>,
    stop: StopReason,
    input: u64,
    output: u64,
    cache_read: u64,
}

impl StreamAsm {
    fn new(model: String, price: (Option<f64>, Option<f64>)) -> StreamAsm {
        StreamAsm {
            model,
            price,
            blocks: BTreeMap::new(),
            stop: StopReason::EndTurn,
            input: 0,
            output: 0,
            cache_read: 0,
        }
    }

    /// Process one SSE event; return `false` to stop (message_stop / refusal).
    fn on_event(
        &mut self,
        event: &eventsource_stream::Event,
        tx: &mpsc::UnboundedSender<Result<StreamEvent, LlmError>>,
    ) -> bool {
        let data: Value = match serde_json::from_str(&event.data) {
            Ok(v) => v,
            Err(_) => return true,
        };
        match event.event.as_str() {
            "message_start" => {
                if let Some(u) = data.pointer("/message/usage") {
                    self.input = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    self.cache_read = u
                        .get("cache_read_input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                }
            }
            "content_block_start" => {
                let idx = data.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let cb = data.get("content_block");
                match cb.and_then(|c| c.get("type")).and_then(Value::as_str) {
                    Some("tool_use") => {
                        let id = cb
                            .and_then(|c| c.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let name = cb
                            .and_then(|c| c.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let _ = tx.unbounded_send(Ok(StreamEvent::ToolUseStart {
                            id: id.clone(),
                            name: name.clone(),
                        }));
                        self.blocks.insert(
                            idx,
                            BlockAsm::Tool {
                                id,
                                name,
                                json: String::new(),
                            },
                        );
                    }
                    _ => {
                        self.blocks.insert(idx, BlockAsm::Text(String::new()));
                    }
                }
            }
            "content_block_delta" => {
                let idx = data.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = data.get("delta");
                match delta.and_then(|d| d.get("type")).and_then(Value::as_str) {
                    Some("text_delta") => {
                        let t = delta
                            .and_then(|d| d.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if let Some(BlockAsm::Text(s)) = self.blocks.get_mut(&idx) {
                            s.push_str(t);
                        }
                        let _ = tx.unbounded_send(Ok(StreamEvent::TextDelta(t.to_string())));
                    }
                    Some("input_json_delta") => {
                        let frag = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let id = match self.blocks.get_mut(&idx) {
                            Some(BlockAsm::Tool { id, json, .. }) => {
                                json.push_str(frag);
                                id.clone()
                            }
                            _ => String::new(),
                        };
                        let _ = tx.unbounded_send(Ok(StreamEvent::ToolInputDelta {
                            id,
                            json_fragment: frag.to_string(),
                        }));
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(s) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    if s == "refusal" {
                        let _ = tx.unbounded_send(Err(LlmError::Refused { category: None }));
                        return false;
                    }
                    self.stop = parse_stop(s);
                }
                if let Some(o) = data.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                    self.output = o;
                }
            }
            "message_stop" => return false,
            _ => {}
        }
        true
    }

    fn finish(self) -> CompletionResponse {
        let content = self
            .blocks
            .into_values()
            .map(|b| match b {
                BlockAsm::Text(text) => ContentBlock::Text { text },
                BlockAsm::Tool { id, name, json } => ContentBlock::ToolUse {
                    id,
                    name,
                    input: serde_json::from_str(&json).unwrap_or(Value::Object(Default::default())),
                },
            })
            .collect();
        let cost = match self.price {
            (None, None) => None,
            (pin, pout) => Some(
                pin.unwrap_or(0.0) * (self.input as f64) / 1_000_000.0
                    + pout.unwrap_or(0.0) * (self.output as f64) / 1_000_000.0,
            ),
        };
        CompletionResponse {
            content,
            stop_reason: self.stop,
            usage: Usage {
                input_tokens: self.input,
                output_tokens: self.output,
                cache_read_tokens: self.cache_read,
                reasoning_tokens: 0,
                cost_usd: cost,
            },
            model: self.model,
        }
    }
}

// -- mapping helpers -------------------------------------------------------

fn message_json(m: &ChatMessage) -> Value {
    let role = match m.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let content: Vec<Value> = m
        .content
        .iter()
        .map(|b| match b {
            ContentBlock::Text { text } => json!({"type": "text", "text": text}),
            ContentBlock::ToolUse { id, name, input } => {
                json!({"type": "tool_use", "id": id, "name": name, "input": input})
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error,
            }),
        })
        .collect();
    json!({"role": role, "content": content})
}

fn tool_choice_json(tc: &ToolChoice) -> Option<Value> {
    Some(match tc {
        ToolChoice::Auto => json!({"type": "auto"}),
        ToolChoice::Required => json!({"type": "any"}),
        ToolChoice::Tool(name) => json!({"type": "tool", "name": name}),
        ToolChoice::None => json!({"type": "none"}),
    })
}

fn parse_block(v: &Value) -> Option<ContentBlock> {
    match v.get("type").and_then(Value::as_str)? {
        "text" => Some(ContentBlock::Text {
            text: v.get("text").and_then(Value::as_str)?.to_string(),
        }),
        "tool_use" => Some(ContentBlock::ToolUse {
            id: v
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            name: v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            input: v.get("input").cloned().unwrap_or(Value::Null),
        }),
        _ => None,
    }
}

fn parse_stop(s: &str) -> StopReason {
    match s {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "tool_use" => StopReason::ToolUse,
        "stop_sequence" => StopReason::StopSequence,
        "refusal" => StopReason::Refusal,
        other => StopReason::Other(other.to_string()),
    }
}

fn effort_str(e: Effort) -> &'static str {
    e.as_str()
}

/// The opus-5 / sonnet-5 flagship generation, which rejects both
/// `temperature`/`top_p` and the old `thinking.budget_tokens` reasoning form.
/// Model ids arrive already stripped of the `anthropic/` route prefix.
fn is_flagship_generation(model: &str) -> bool {
    let m = model.strip_prefix("claude-").unwrap_or(model);
    m.starts_with("opus-5") || m.starts_with("sonnet-5")
}

/// Whether the model accepts `temperature`/`top_p` (false for the flagship).
fn model_accepts_sampling(model: &str) -> bool {
    !is_flagship_generation(model)
}

/// Approximate a `budget_tokens` reasoning ceiling as an `output_config.effort`
/// level for the flagship generation, which rejects the raw budget form.
fn effort_for_budget(n: u32) -> Effort {
    if n <= 2048 {
        Effort::Low
    } else if n <= 8192 {
        Effort::Medium
    } else {
        Effort::High
    }
}

fn req_model_fallback() -> String {
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flagship_sampling_gated_off() {
        assert!(!model_accepts_sampling("claude-opus-5"));
        assert!(!model_accepts_sampling("claude-opus-5-20260101"));
        assert!(!model_accepts_sampling("claude-sonnet-5"));
        assert!(model_accepts_sampling("claude-haiku-4-5"));
        assert!(model_accepts_sampling("claude-3-5-sonnet"));
    }

    #[test]
    fn flagship_maxtokens_maps_to_effort_not_budget() {
        // Flagship: reasoning_max_tokens becomes output_config.effort.
        let cfg = ProviderConfig {
            kind: crate::config::ProviderKind::Anthropic,
            base_url: "https://example.invalid".into(),
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
        };
        let provider = AnthropicProvider::new("anthropic", &cfg, "k".into()).unwrap();

        let mut req = CompletionRequest::simple("hi");
        req.model = "claude-opus-5".into();
        req.reasoning = Some(ReasoningConfig::MaxTokens(4000));
        let body = provider.body(&req, false);
        assert!(
            body.get("thinking").is_none(),
            "flagship must not send thinking.budget_tokens: {body}"
        );
        assert_eq!(body["output_config"]["effort"], "medium");

        // Prior generation: the raw budget_tokens form is still used.
        req.model = "claude-3-5-sonnet".into();
        let body = provider.body(&req, false);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 4000);
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn effort_for_budget_buckets() {
        assert_eq!(effort_for_budget(1000), Effort::Low);
        assert_eq!(effort_for_budget(4000), Effort::Medium);
        assert_eq!(effort_for_budget(20000), Effort::High);
    }
}
