//! A network-free provider for testing [`crate::service::LlmService`] and
//! (later) worldgen. Scriptable canned responses + error injection, plus a
//! record of every request received so tests can assert routing and request
//! shape.
//!
//! Available whenever the `mock` feature is on (default). It pulls no HTTP
//! dependencies.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use futures::stream::{self, StreamExt};

use crate::provider::{LlmError, LlmProvider, ProviderCaps, ProviderId};
use crate::types::{
    CompletionRequest, CompletionResponse, ContentBlock, LlmStream, StopReason, StreamEvent, Usage,
};

/// A scriptable fake provider.
pub struct MockProvider {
    id: ProviderId,
    caps: ProviderCaps,
    inner: Mutex<Inner>,
}

struct Inner {
    scripted: VecDeque<Result<CompletionResponse, LlmError>>,
    default: Result<CompletionResponse, LlmError>,
    calls: Vec<CompletionRequest>,
}

impl std::fmt::Debug for MockProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = self.inner.lock().expect("mock lock");
        write!(
            f,
            "MockProvider(id={}, calls={}, queued={})",
            self.id,
            g.calls.len(),
            g.scripted.len()
        )
    }
}

impl MockProvider {
    /// A mock registered under `id`, defaulting to an empty "ok" response.
    #[must_use]
    pub fn new(id: impl Into<ProviderId>) -> MockProvider {
        MockProvider {
            id: id.into(),
            caps: ProviderCaps::default(),
            inner: Mutex::new(Inner {
                scripted: VecDeque::new(),
                default: Ok(ok_response("", Usage::default())),
                calls: Vec::new(),
            }),
        }
    }

    /// Override the reported capabilities.
    #[must_use]
    pub fn with_caps(mut self, caps: ProviderCaps) -> MockProvider {
        self.caps = caps;
        self
    }

    /// Queue the next response.
    pub fn push_response(&self, resp: CompletionResponse) -> &MockProvider {
        self.inner
            .lock()
            .expect("mock lock")
            .scripted
            .push_back(Ok(resp));
        self
    }

    /// Queue the next outcome as an error.
    pub fn push_error(&self, err: LlmError) -> &MockProvider {
        self.inner
            .lock()
            .expect("mock lock")
            .scripted
            .push_back(Err(err));
        self
    }

    /// Queue an ok text response with explicit usage.
    pub fn push_ok(&self, text: impl Into<String>, usage: Usage) -> &MockProvider {
        self.push_response(ok_response(text, usage))
    }

    /// Set the response returned once the scripted queue is drained.
    pub fn set_default(&self, outcome: Result<CompletionResponse, LlmError>) -> &MockProvider {
        self.inner.lock().expect("mock lock").default = outcome;
        self
    }

    /// How many times the provider was called.
    #[must_use]
    pub fn call_count(&self) -> usize {
        self.inner.lock().expect("mock lock").calls.len()
    }

    /// Every request received, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<CompletionRequest> {
        self.inner.lock().expect("mock lock").calls.clone()
    }

    /// The most recent request received, if any.
    #[must_use]
    pub fn last_request(&self) -> Option<CompletionRequest> {
        self.inner.lock().expect("mock lock").calls.last().cloned()
    }

    fn next_outcome(&self, req: &CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut g = self.inner.lock().expect("mock lock");
        g.calls.push(req.clone());
        match g.scripted.pop_front() {
            Some(o) => o,
            None => g.default.clone(),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for MockProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn caps(&self) -> ProviderCaps {
        self.caps
    }

    async fn complete(
        &self,
        req: &CompletionRequest,
        _timeout: Duration,
    ) -> Result<CompletionResponse, LlmError> {
        self.next_outcome(req)
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
        _timeout: Duration,
    ) -> Result<LlmStream, LlmError> {
        match self.next_outcome(req) {
            Ok(resp) => {
                let mut events: Vec<Result<StreamEvent, LlmError>> = Vec::new();
                let text = resp.text();
                if !text.is_empty() {
                    events.push(Ok(StreamEvent::TextDelta(text)));
                }
                events.push(Ok(StreamEvent::Done(resp)));
                Ok(stream::iter(events).boxed())
            }
            Err(e) => Ok(stream::once(async move { Err(e) }).boxed()),
        }
    }
}

/// Build an ok text [`CompletionResponse`].
#[must_use]
pub fn ok_response(text: impl Into<String>, usage: Usage) -> CompletionResponse {
    let text = text.into();
    let content = if text.is_empty() {
        Vec::new()
    } else {
        vec![ContentBlock::text(text)]
    };
    CompletionResponse {
        content,
        stop_reason: StopReason::EndTurn,
        usage,
        model: "mock".to_string(),
    }
}

/// Build a [`Usage`] with token counts and an optional cost.
#[must_use]
pub fn usage(input: u64, output: u64, cost_usd: Option<f64>) -> Usage {
    Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: 0,
        reasoning_tokens: 0,
        cost_usd,
    }
}
