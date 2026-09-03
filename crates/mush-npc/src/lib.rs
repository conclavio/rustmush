//! # mush-npc — LLM NPC agents
//!
//! The NPC agent runtime (design 03 §6): an [`ObjKind::Npc`](mush_core::ObjKind)
//! object with an [`AgentProfile`] becomes an LLM-driven character. Built as the
//! `npc` [`GameModule`](mush_core::GameModule) on the merged module API plus
//! [`mush_llm`]'s `LlmService` and the `ModuleAsync` re-entry seam — the exact
//! two-phase pattern `mush-worldgen` uses (treaty ruling 3): snapshot context
//! synchronously on the world loop, run the LLM turn **off** it, re-enter as
//! typed actions.
//!
//! ## The security boundary is the action allowlist, not the prompt
//!
//! Players **will** try to jailbreak NPCs. The design is built so a *fully
//! jailbroken* NPC can only ever emit one of the typed [`NpcAction`]s
//! (say / pose / move / look_at / remember / use_ability / do_nothing). There
//! is deliberately **no** action that runs a command line, reads another
//! player's data, spends currency, or escalates. Concretely:
//!
//! - Every action from the model is re-parsed into a typed [`NpcAction`]
//!   ([`action::parse_actions`]) and re-validated in the world task before it
//!   applies — the room may have changed during the call, so a `Move` through a
//!   now-locked exit fails exactly like a player's.
//! - An **invented** tool name (a jailbreak) collapses to
//!   [`NpcAction::UseAbility`], which [`Ruleset::invoke_ability`](mush_ruleset::Ruleset::invoke_ability)
//!   rejects — never a command.
//! - Prompts carry **no secrets**: only the NPC's own profile and public room
//!   state (design 03 §7 rule 2). Player speech is delimited and `<`-escaped and
//!   framed as in-fiction speech ([`prompt`]).
//! - Output is hygiene-checked: length-capped, control/ANSI-stripped, and
//!   rendered through the **normal** say/pose formatter with the NPC's true
//!   name — it cannot forge another speaker or a system line.
//! - Every turn is audited by the `LlmService` (actor + npc attribution).
//! - On any budget trip / refusal / disabled service the NPC degrades to a
//!   profile fallback line (never an error to players) plus a one-time staff
//!   notice.
//!
//! ## Runtime shape (no tokio tasks)
//!
//! `mush-core` is sync and owns the world, so the design's "per-NPC task"
//! becomes per-NPC state in a module state slot ([`NpcHostState`]) driven by the
//! module's tick + observers ([`module`]):
//!
//! - **Observers** turn [`WorldEvent`](mush_core::module::WorldEvent)s into
//!   [`Stimulus`]es and enqueue them — but only when a connected player shares
//!   the room, so an NPC alone burns zero tokens.
//! - The **tick** opens a coalescing window (rapid chatter → one turn), enforces
//!   the tier cooldown and one-in-flight, assembles a [`TurnSnapshot`], and
//!   defers one [`Effect::ModuleAsync`](mush_core::Effect::ModuleAsync).
//! - [`run_npc_turn_async`] runs the bounded tool loop off-loop and re-enters as
//!   a [`TurnOutcome`], applied by the module's
//!   [`AsyncCompletion`](mush_core::AsyncCompletion).
//!
//! The driver ([`mushd`]) routes the `npc` `Effect::ModuleAsync` to
//! [`run_npc_turn_async`] (as it does worldgen) and supplies the
//! [`AgentGrounding`](snapshot::AgentGrounding) adapter over the loaded ruleset
//! (sheet grounding + on-loop ability execution).
//!
//! ## The `wod_roll` UseAbility contract (for the grounding ruleset)
//!
//! `UseAbility` executes **on the world loop during re-entry**, through
//! [`AgentGrounding::invoke_ability`](snapshot::AgentGrounding::invoke_ability),
//! which the driver backs with
//! [`Ruleset::invoke_ability`](mush_ruleset::Ruleset::invoke_ability) — that
//! method takes `&mut World` + `&mut Outbox`, so it has everything it needs to
//! ground the roll in real game state. The off-loop turn does **not** resolve
//! the ability; it records a [`NpcAction::UseAbility`] and hands the model a
//! placeholder tool result ("the engine will resolve it") so the model narrates
//! in character without stating a mechanical outcome it cannot know.
//!
//! On re-entry the completion calls `invoke_ability(w, out, npc, name, input)`.
//! The WoD `wod_roll` implementation (in `ruleset-wod`):
//!
//! - reads the NPC's **own** `sheet:wod` document;
//! - resolves `pool_expr` against traits actually on that sheet (an over-claim
//!   fence — a jailbroken model cannot manufacture a pool it doesn't have);
//! - applies wound penalty, an optional recorded specialty, and an optional
//!   Willpower spend from the NPC's own pool, plus a bounded situational
//!   `modifier` (±3);
//! - rolls `Owod20Pool` on the **shared server RNG** and records it in the roll
//!   log, exactly as the `+roll` command does;
//! - emits the classic roll line to the room and the `wod.roll` panel itself;
//!   and
//! - returns an [`AbilityOutcome`](mush_ruleset::AbilityOutcome) whose `summary`
//!   is an in-fiction narration hint and whose `data` carries the polarity /
//!   successes so the agent layer can (later) validate the model's narration.
//!
//! Because the engine computes and emits the outcome, the NPC can never forge a
//! roll — the model only requests it. See `ruleset-wod`'s `agent` module for the
//! stable `wod_roll` tool schema (`pool_expr`, `difficulty`, `specialty`,
//! `willpower`, `modifier`).
//!
//! ## Not built (documented seams)
//!
//! - **Embedding / vector recall** is the M10 [`MemoryRecall`](memory::MemoryRecall)
//!   seam; recall v1 ([`LongTermStore::recall`](memory::LongTermStore::recall)) is
//!   name-match only.
//! - **Ambient timers** (`Stimulus::AmbientTimer`) are modeled but not yet
//!   scheduled; the tick fires only on real stimuli / pending summaries.

mod action;
mod intel;
mod knowledge;
mod memory;
mod module;
mod profile;
mod prompt;
pub mod query;
mod runtime;
mod snapshot;
mod stimulus;
mod turn;

pub use action::{
    MAX_ACTIONS, MAX_ROUND_TRIPS, NpcAction, SAY_CAP, action_tools, parse_actions, sanitize_line,
};
pub use intel::{INTEL_CAP, IntelKind, IntelRecord, IntelStore, intel_key};
pub use knowledge::{
    Familiarity, KNOWLEDGE_ATTR, LoreEntry, LorePool, LorePools, NpcKnowledge, Secrecy,
    familiarity_of, pool_key,
};
pub use memory::{
    BUFFER_CAP, LongTermStore, MemoryBuffer, MemoryEvent, MemoryRecall, MemoryRecord, record_key,
};
pub use module::NpcModule;
pub use profile::{
    AgentProfile, KnowledgeBounds, NS, NpcTier, PROFILE_ATTR, SUMMARY_ATTR, THINKING_FEED,
};
pub use prompt::{ENGINE_FRAME, system_prompt, user_prompt};
pub use query::IntelListQuery;
pub use runtime::{NpcConfig, NpcHostState, NpcRuntime};
pub use snapshot::{
    AgentGrounding, AppliedAction, NoGrounding, PresentPlayer, RecalledMemory, SceneView,
    StimulusLine, TurnKind, TurnOutcome, TurnSnapshot,
};
pub use stimulus::{SpeechKind, Stimulus};
pub use turn::run_npc_turn_async;
