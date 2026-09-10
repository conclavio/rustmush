//! The bounded **tool loop** (14-spatial-implementation-plan §3.4, S3): one
//! implementation of "call the model, answer its tool calls, call again" that an
//! NPC turn and the worldgen planner both run on.
//!
//! The loop owns the mechanics — the conversation, the round bound, the echo of
//! the assistant turn (only the tool-use blocks that were answered, so a dropped
//! call never leaves an unmatched id for the next request to 400 on), one repair
//! round for a reply that carried nothing usable, and the **per-run cost ceiling**
//! — and a [`ToolHost`] owns the meaning: what each tool does, when the run is
//! done, what a text-only reply is worth. The host never sees the service; the
//! loop never interprets a tool.
//!
//! Every call goes through [`LlmService::call`], so the kill switch, the routes,
//! the daily budgets and the audit row apply per call exactly as they do to a
//! single call. What this module adds is the **run**: `max_rounds` provider
//! calls at most, and a [`RunBudget`] whose `cap_usd` is checked before every
//! call against what the run has already spent — a pre-check like every other
//! budget, so a run stops *before* the call that would run past the cap, and
//! whose `target_usd` is advice the caller tells the model and nothing enforces.
//! Cost is the provider's own `usage.cost_usd`; a provider that reports none
//! spends nothing against the cap, which is the honest answer when there is no
//! price to count.

use crate::provider::LlmError;
use crate::service::{CallCtx, LlmService};
use crate::types::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentBlock, Role, Sampling, ToolChoice,
    ToolDef,
};

/// The per-run cost ceiling (plan §3.4). `cap_usd` is enforced before every
/// call; `target_usd` is told to the model by the caller and never enforced.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunBudget {
    /// The hard ceiling, USD: the loop stops before a call once the run's spend
    /// has reached it. `None` is uncapped.
    pub cap_usd: Option<f64>,
    /// The soft target, USD, for the caller's prompt. Never enforced.
    pub target_usd: Option<f64>,
}

/// The bounds of one run.
#[derive(Clone, Debug, PartialEq)]
pub struct RunLimits {
    /// The most provider calls the run makes, repairs included.
    pub max_rounds: usize,
    /// The cost ceiling.
    pub budget: RunBudget,
    /// `max_tokens` on every request; the route's own ceiling overrides it in
    /// the service.
    pub max_tokens: u32,
}

/// What a host answers one tool call with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolReply {
    /// The tool result the model reads.
    pub content: String,
    /// Whether the call failed (drives the model's self-correction).
    pub is_error: bool,
    /// Whether this result invites another round: the model asked for something
    /// and may now act on the answer. A round continues only when **every**
    /// answered call in it invites one; any other round ends the run.
    pub more: bool,
}

/// What a host makes of a reply that carried no tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextVerdict {
    /// The run is over: the host took what it needed, or there was nothing.
    Done,
    /// Send this back as the next user turn, once per run, and try again.
    Repair(String),
    /// Send this back as the next user turn and go on (counts as a round).
    Continue(String),
}

/// The meaning of a run: what each tool does and when the run is done.
pub trait ToolHost {
    /// Answer one tool call. `None` declines it: the block is left out of the
    /// echo and the round goes on without it.
    fn call(&mut self, name: &str, input: &serde_json::Value) -> Option<ToolReply>;

    /// A reply with no tool call at all.
    fn on_text(&mut self, reply: &CompletionResponse) -> TextVerdict;

    /// Whether the host has what it needs. Checked before every call and
    /// between the tool calls of a round; `true` stops the run.
    fn finished(&self) -> bool {
        false
    }
}

/// Why a run stopped.
#[derive(Debug)]
pub enum StopCause {
    /// The host was finished, or a round ended with no invitation to go on.
    Done,
    /// A reply carried nothing usable and the run's one repair was spent.
    NoTool,
    /// `max_rounds` calls were made.
    Rounds,
    /// The run's spend reached `cap_usd` before a call.
    Cap,
    /// The service refused or failed: the kill switch, a daily budget, a
    /// refusal, a transport failure. The run keeps what earlier rounds gave it.
    Error(LlmError),
}

impl StopCause {
    /// Whether the run ended on the host's terms rather than a bound or a fault.
    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(self, StopCause::Done)
    }
}

/// What a run came to.
#[derive(Debug)]
pub struct RunReport {
    /// Provider calls made, repairs included.
    pub rounds: usize,
    /// The run's spend, USD, as the provider reported it.
    pub cost_usd: f64,
    /// Input tokens over the run.
    pub input_tokens: u64,
    /// Output tokens over the run.
    pub output_tokens: u64,
    /// Why it stopped.
    pub stopped: StopCause,
    /// The model that served the last reply, if any.
    pub model: String,
    /// The text of the last reply, if any.
    pub last_text: String,
}

/// Everything a run starts from.
#[derive(Clone, Debug)]
pub struct RunSpec {
    /// The metered context every call is made under.
    pub ctx: CallCtx,
    /// The system prompt.
    pub system: String,
    /// The opening user turn.
    pub opening: String,
    /// The tools the model may call.
    pub tools: Vec<ToolDef>,
    /// The tool policy.
    pub tool_choice: ToolChoice,
    /// The bounds.
    pub limits: RunLimits,
}

/// Run one bounded tool loop against `llm` and report.
///
/// The host is `Send` so the future is: the drivers spawn these runs on the
/// runtime, off the world loop.
///
/// The loop makes at most `limits.max_rounds` calls. Before each it asks the host
/// whether it is finished and checks the run's spend against `cap_usd`. A reply
/// with tool calls has each answered by the host; the round continues when every
/// answer invites more, else the run is done. A reply with no tool call is the
/// host's to judge: done, one repair, or a nudge.
pub async fn run<H: ToolHost + Send + ?Sized>(
    llm: &LlmService,
    spec: RunSpec,
    host: &mut H,
) -> RunReport {
    let RunSpec {
        ctx,
        system,
        opening,
        tools,
        tool_choice,
        limits,
    } = spec;
    let mut report = RunReport {
        rounds: 0,
        cost_usd: 0.0,
        input_tokens: 0,
        output_tokens: 0,
        stopped: StopCause::Rounds,
        model: String::new(),
        last_text: String::new(),
    };
    let mut messages = vec![ChatMessage::user(opening)];
    let mut repaired = false;

    for round in 0..limits.max_rounds {
        if host.finished() {
            report.stopped = StopCause::Done;
            return report;
        }
        if let Some(cap) = limits.budget.cap_usd
            && report.cost_usd >= cap
        {
            report.stopped = StopCause::Cap;
            return report;
        }
        let req = CompletionRequest {
            model: String::new(),
            system: Some(system.clone()),
            messages: messages.clone(),
            tools: tools.clone(),
            tool_choice: tool_choice.clone(),
            max_tokens: limits.max_tokens,
            sampling: Sampling::default(),
            stop_sequences: Vec::new(),
            reasoning: None,
        };
        let reply = match llm.call(ctx, req).await {
            Ok(reply) => reply,
            Err(error) => {
                report.stopped = StopCause::Error(error);
                return report;
            }
        };
        report.rounds += 1;
        report.input_tokens = report.input_tokens.saturating_add(reply.usage.input_tokens);
        report.output_tokens = report
            .output_tokens
            .saturating_add(reply.usage.output_tokens);
        if let Some(cost) = reply.usage.cost_usd {
            report.cost_usd += cost;
        }
        report.model = reply.model.clone();
        report.last_text = reply.text();

        let tool_uses: Vec<(String, String, serde_json::Value)> = reply
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.clone(), name.clone(), input.clone()))
                }
                _ => None,
            })
            .collect();

        if tool_uses.is_empty() {
            match host.on_text(&reply) {
                TextVerdict::Done => {
                    report.stopped = StopCause::Done;
                    return report;
                }
                TextVerdict::Repair(prompt) => {
                    if repaired {
                        report.stopped = StopCause::NoTool;
                        return report;
                    }
                    repaired = true;
                    messages.push(ChatMessage::assistant(report.last_text.clone()));
                    messages.push(ChatMessage::user(prompt));
                }
                TextVerdict::Continue(prompt) => {
                    messages.push(ChatMessage::assistant(report.last_text.clone()));
                    messages.push(ChatMessage::user(prompt));
                }
            }
            continue;
        }

        let mut results: Vec<ContentBlock> = Vec::new();
        let mut answered: Vec<String> = Vec::new();
        let mut all_more = true;
        for (id, name, input) in &tool_uses {
            if host.finished() {
                break;
            }
            let Some(ToolReply {
                content,
                is_error,
                more,
            }) = host.call(name, input)
            else {
                continue;
            };
            all_more &= more;
            answered.push(id.clone());
            results.push(ContentBlock::ToolResult {
                tool_use_id: id.clone(),
                content,
                is_error,
            });
        }
        let more = !answered.is_empty()
            && all_more
            && round.saturating_add(1) < limits.max_rounds
            && !host.finished();
        if !more {
            report.stopped = StopCause::Done;
            return report;
        }
        // Echo back only what was answered: a declined call has no result, and
        // an unmatched tool-use id is a request the next call refuses.
        let echo: Vec<ContentBlock> = reply
            .content
            .iter()
            .filter(|block| match block {
                ContentBlock::ToolUse { id, .. } => answered.contains(id),
                _ => true,
            })
            .cloned()
            .collect();
        messages.push(ChatMessage {
            role: Role::Assistant,
            content: echo,
        });
        messages.push(ChatMessage {
            role: Role::User,
            content: results,
        });
    }
    report
}
