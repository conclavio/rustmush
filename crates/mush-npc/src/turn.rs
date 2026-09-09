//! The off-loop agent turn (design 03 §6.3–§6.4, §8): the **only** async code in
//! the crate. It holds no world reference — it consumes a serialized
//! [`TurnSnapshot`] and produces a serialized [`TurnOutcome`], exactly the
//! worldgen re-entry shape (treaty ruling 3).
//!
//! It runs the bounded tool loop (max [`MAX_ROUND_TRIPS`] round-trips, max
//! [`MAX_ACTIONS`] actions) on `mush_llm::agent` — the one loop the worldgen
//! planner runs on too (S3) — supports a JSON-actions fallback with one repair
//! for weak-tool models, resolves `LookAt` from the scene snapshot into a real
//! tool result, and degrades to a profile fallback line on any budget trip /
//! refusal / disabled service — never an error to players.
//!
//! `UseAbility` is **not** resolved here: the ruleset's `invoke_ability` runs
//! on the world loop (it needs the sheet, the shared RNG, and the outbox), so an
//! ability call is recorded as a typed action and executed during re-entry.

use mush_llm::agent::{self, RunBudget, RunLimits, RunSpec, StopCause, TextVerdict, ToolHost, ToolReply};
use mush_llm::{
    BudgetLimit, CallCtx, ChatMessage, CompletionRequest, CompletionResponse, LlmError,
    LlmService, Sampling, ToolDef, Priority,
};

use crate::action::NpcAction;
use crate::action::{
    MAX_ACTIONS, MAX_ROUND_TRIPS, action_tools, actions_from_tool, clean_action, parse_actions,
    tool_choice,
};
use crate::prompt::{system_prompt, user_prompt};
use crate::snapshot::{AppliedAction, TurnKind, TurnOutcome, TurnSnapshot};

/// Run one NPC turn end-to-end and return the serialized [`TurnOutcome`]. The
/// driver calls this off the world loop with its shared [`LlmService`];
/// `payload` is a serialized [`TurnSnapshot`].
pub async fn run_npc_turn_async(llm: &LlmService, payload: &str) -> String {
    let outcome = match serde_json::from_str::<TurnSnapshot>(payload) {
        Ok(snap) => match snap.validate() {
            Ok(()) => run_turn(llm, &snap).await,
            Err(error) => {
                tracing::error!(npc = snap.npc.0, %error, "npc: invalid bounded turn snapshot");
                TurnOutcome::Nothing {
                    npc: snap.npc,
                    reason: error.to_string(),
                }
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "npc: undecodable turn snapshot");
            TurnOutcome::Nothing {
                npc: mush_core::ObjId(0),
                reason: format!("internal error decoding turn: {e}"),
            }
        }
    };
    serde_json::to_string(&outcome).unwrap_or_else(|e| {
        tracing::error!(error = %e, "npc: unserializable turn outcome");
        String::from(r#"{"status":"nothing","npc":0,"reason":"internal error"}"#)
    })
}

async fn run_turn(llm: &LlmService, snap: &TurnSnapshot) -> TurnOutcome {
    match snap.kind {
        TurnKind::Summarize => run_summarize(llm, snap).await,
        TurnKind::Dialogue => run_dialogue(llm, snap).await,
    }
}

/// A dialogue/action deliberation on the shared bounded tool loop
/// (`mush_llm::agent`, S3): the loop owns the rounds, the answered-only echo and
/// the one repair; [`DialogueHost`] owns what each tool means to an NPC.
async fn run_dialogue(llm: &LlmService, snap: &TurnSnapshot) -> TurnOutcome {
    let feature = snap.profile.tier.dialogue_feature();
    let mut ctx = CallCtx::new(feature, Priority::Interactive).with_npc(snap.npc);
    if let Some(actor) = snap.actor {
        ctx = ctx.with_actor(actor);
    }
    // The NPC's per-agent daily budget override (`@agent/set <npc>/daily_tokens`)
    // becomes the call's per-NPC ceiling — the documented containment lever.
    if let Some(tokens) = snap.profile.daily_tokens {
        ctx = ctx.with_npc_limit(BudgetLimit::tokens(tokens));
    }

    let spec = RunSpec {
        ctx,
        system: system_prompt(snap),
        opening: user_prompt(snap),
        tools: all_tools(snap),
        tool_choice: tool_choice(),
        limits: RunLimits {
            max_rounds: MAX_ROUND_TRIPS,
            // An NPC turn is bounded by its daily budgets, not a per-run cap.
            budget: RunBudget::default(),
            max_tokens: 1024,
        },
    };
    let mut host = DialogueHost {
        snap,
        collected: Vec::new(),
    };
    let report = agent::run(llm, spec, &mut host).await;
    match report.stopped {
        // Any budget trip / refusal / disabled / transport failure → a
        // graceful fallback line (design 03 §6.6 / §7 rule 6).
        StopCause::Error(error) => degrade(snap, &error),
        StopCause::Done | StopCause::NoTool | StopCause::Rounds | StopCause::Cap => {
            finalize(snap, host.collected)
        }
    }
}

/// What the tools mean to an NPC turn: a built-in tool is its typed action, any
/// other name is a ruleset ability, and only `look_at` returns something worth
/// another round.
struct DialogueHost<'a> {
    snap: &'a TurnSnapshot,
    collected: Vec<AppliedAction>,
}

impl ToolHost for DialogueHost<'_> {
    fn call(&mut self, name: &str, input: &serde_json::Value) -> Option<ToolReply> {
        let mut lines: Vec<String> = Vec::new();
        let mut is_error = false;
        let mut more = true;
        for raw in actions_from_tool(name, input) {
            if self.collected.len() >= MAX_ACTIONS {
                break;
            }
            let Some(action) = clean_action(raw) else {
                continue;
            };
            let (text, failed) = tool_result_for(self.snap, &action);
            // Continue only for a pure information-gathering call (LookAt);
            // anything else ends the turn once the round is answered.
            more &= action.returns_result();
            is_error |= failed;
            lines.push(text);
            self.collected.push(AppliedAction { action });
        }
        // A call whose every action was dropped (an empty target) has no
        // result: leaving it out of the echo keeps the next request valid.
        (!lines.is_empty()).then(|| ToolReply {
            content: lines.join("\n"),
            is_error,
            more,
        })
    }

    fn on_text(&mut self, reply: &CompletionResponse) -> TextVerdict {
        // The JSON fallback for weak-tool models, with one repair.
        let (actions, needs_repair) = parse_actions(reply);
        if !actions.is_empty() {
            for action in actions {
                if self.collected.len() >= MAX_ACTIONS {
                    break;
                }
                self.collected.push(AppliedAction { action });
            }
            return TextVerdict::Done;
        }
        if needs_repair {
            TextVerdict::Repair(
                "That was not a valid action. Reply by calling a tool (say/pose/move/look_at/\
                 remember/do_nothing), or with ONLY a JSON array like \
                 [{\"action\":\"say\",\"text\":\"...\"}]."
                    .to_string(),
            )
        } else {
            TextVerdict::Done
        }
    }

    fn finished(&self) -> bool {
        self.collected.len() >= MAX_ACTIONS
    }
}

/// Build the final outcome from the collected actions.
fn finalize(snap: &TurnSnapshot, collected: Vec<AppliedAction>) -> TurnOutcome {
    // Drop do-nothings; if everything was a no-op, the turn is silent.
    let meaningful: Vec<AppliedAction> = collected
        .into_iter()
        .filter(|a| !matches!(a.action, NpcAction::DoNothing { .. }))
        .collect();
    if meaningful.is_empty() {
        return TurnOutcome::Nothing {
            npc: snap.npc,
            reason: "do_nothing".to_string(),
        };
    }
    TurnOutcome::Ok {
        npc: snap.npc,
        actions: meaningful,
    }
}

/// The tool result to hand back for one action. `LookAt` is resolved from the
/// scene snapshot; `UseAbility` is acknowledged (it executes on the world loop
/// during re-entry); everything else is fire-and-forget `"ok"`.
fn tool_result_for(snap: &TurnSnapshot, action: &NpcAction) -> (String, bool) {
    match action {
        NpcAction::LookAt { target } => look_at(snap, target),
        NpcAction::UseAbility { name, .. } => (
            format!(
                "You attempt {name}. The engine will resolve it and show the result in the scene; \
                 narrate in character but do not state the mechanical outcome."
            ),
            false,
        ),
        _ => ("ok".to_string(), false),
    }
}

/// Resolve a `look_at` from the scene snapshot (no world access).
fn look_at(snap: &TurnSnapshot, target: &str) -> (String, bool) {
    let t = target.trim();
    if t.eq_ignore_ascii_case("room")
        || t.eq_ignore_ascii_case("here")
        || t.eq_ignore_ascii_case(&snap.scene.room_name)
    {
        return (
            if snap.scene.room_desc.is_empty() {
                snap.scene.room_name.clone()
            } else {
                snap.scene.room_desc.clone()
            },
            false,
        );
    }
    for p in &snap.scene.present {
        if p.name.eq_ignore_ascii_case(t) {
            let d = if p.short_desc.is_empty() {
                format!("{} is here.", p.name)
            } else {
                format!("{}: {}", p.name, p.short_desc)
            };
            return (d, false);
        }
    }
    (format!("You don't see \"{t}\" here."), true)
}

/// A memory-fold turn: one cheap `NpcSummarize` call folding the oldest events
/// into the summary.
async fn run_summarize(llm: &LlmService, snap: &TurnSnapshot) -> TurnOutcome {
    let mut ctx =
        CallCtx::new(mush_llm::LlmFeature::NpcSummarize, Priority::Background).with_npc(snap.npc);
    // Housekeeping spends against the same per-NPC ceiling as dialogue.
    if let Some(tokens) = snap.profile.daily_tokens {
        ctx = ctx.with_npc_limit(BudgetLimit::tokens(tokens));
    }
    let req = CompletionRequest {
        model: String::new(),
        system: Some(
            "You maintain a compact third-person memory summary for a game NPC.".to_string(),
        ),
        messages: vec![ChatMessage::user(user_prompt(snap))],
        tools: Vec::new(),
        tool_choice: mush_llm::ToolChoice::None,
        max_tokens: 768,
        sampling: Sampling::default(),
        stop_sequences: Vec::new(),
        reasoning: None,
    };
    match llm.call(ctx, req).await {
        Ok(resp) => {
            let summary =
                crate::action::sanitize_line(&resp.text(), crate::memory::MAX_MEMORY_SUMMARY_CHARS);
            let summary = if summary.trim().is_empty() {
                snap.summary.clone()
            } else {
                summary
            };
            TurnOutcome::Summary {
                npc: snap.npc,
                summary,
            }
        }
        // A failed summarize is non-fatal: keep the old summary (the old events
        // are already dropped from the live buffer).
        Err(e) => {
            tracing::warn!(npc = snap.npc.0, error = %e, "npc: summarize failed; keeping old summary");
            TurnOutcome::Summary {
                npc: snap.npc,
                summary: snap.summary.clone(),
            }
        }
    }
}

/// Turn any call error into a fallback-line degradation.
fn degrade(snap: &TurnSnapshot, err: &LlmError) -> TurnOutcome {
    let line = snap.profile.fallback_line(snap.seed);
    TurnOutcome::Degraded {
        npc: snap.npc,
        line,
        reason: format!("{err}"),
    }
}

/// The action allowlist tools plus the NPC's ruleset ability tools.
fn all_tools(snap: &TurnSnapshot) -> Vec<ToolDef> {
    let mut tools = action_tools();
    if let Some(ap) = &snap.agent_profile {
        for t in &ap.tools {
            tools.push(ToolDef {
                name: t.name.to_string(),
                description: t.description.clone(),
                input_schema: t.params_schema.clone(),
                strict: false,
            });
        }
    }
    tools
}
