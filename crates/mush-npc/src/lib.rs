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
//! - A memory action may cite only the engine-issued experience and belief IDs
//!   shown in its captured turn. Later arrivals and omitted recollections cannot
//!   become evidence for an already running response.
//! - An **invented** tool name (a jailbreak) collapses to
//!   [`NpcAction::UseAbility`], which [`Ruleset::invoke_ability`](mush_ruleset::Ruleset::invoke_ability)
//!   rejects — never a command.
//! - Prompts carry **no secrets**: only the NPC's own profile and public room
//!   state (design 03 §7 rule 2). Player speech is delimited and `<`-escaped and
//!   framed as in-fiction speech (`prompt`).
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
//! module's tick + observers (`module`):
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
//! The driver (`mush-server`) routes the `npc` `Effect::ModuleAsync` to
//! [`run_npc_turn_async`] (as it does worldgen) and supplies the
//! [`AgentGrounding`] adapter over the loaded ruleset
//! (sheet grounding + on-loop ability execution).
//!
//! ## The `wod_roll` UseAbility contract (for the grounding ruleset)
//!
//! `UseAbility` executes **on the world loop during re-entry**, through
//! [`AgentGrounding::invoke_ability`],
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
//! - **Embedding / vector recall** is the M10 [`MemoryRecall`]
//!   seam; legacy recall uses name matches, while personal experience/belief
//!   recall uses bounded name/topic matching and deterministic aging.
//! - **Ambient timers** (`Stimulus::AmbientTimer`) are modeled but not yet
//!   scheduled; the tick fires only on real stimuli / pending summaries.

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

mod action;
pub mod content;
pub mod decision;
pub mod experience;
mod intel;
pub mod interface;
mod knowledge;
mod memory;
mod module;
mod private_lore;
mod profile;
mod prompt;
pub mod query;
pub mod relationships;
mod runtime;
mod snapshot;
mod stimulus;
mod turn;

pub use action::{
    MAX_ACTIONS, MAX_ROUND_TRIPS, NpcAction, SAY_CAP, action_tools, parse_actions, sanitize_line,
};
pub use decision::{
    DECIDE_VERB, IDLE_VERB, MAX_DECISION_REQUESTS, NPC_CONTEXT_SCHEMA, NPC_CONTROLLER_KEY,
    NPC_DECISION_LEASE_SECS, NPC_WAKE_ADAPTER, NpcDecisionHandles, NpcDecisionRequest, TurnReceipt,
    receipt_key, select_idle,
};
pub use experience::{
    Belief, Experience, ExperienceKind, PersonalMemoryRecord, PersonalMemoryStore, PersonalRecall,
    mind_key,
};
pub use intel::{
    INTEL_CAP, IntelKind, IntelRecord, IntelStore, IntelValidationError, MAX_INTEL_DETAIL_BYTES,
    MAX_INTEL_DOCUMENT_BYTES, MAX_INTEL_PROJECTION_BYTES, MAX_INTEL_PROJECTION_RECORDS,
    MAX_INTEL_PROVENANCE_BYTES, MAX_INTEL_READABLE_POOLS, MAX_INTEL_RECORD_BYTES,
    MAX_INTEL_ROOM_NAME_BYTES, MAX_INTEL_SUBJECT_NAME_BYTES, intel_key,
};
pub use knowledge::{
    Familiarity, KNOWLEDGE_ATTR, LoreEntry, LorePool, LorePools, LoreValidationError,
    MAX_LORE_DOCUMENT_BYTES, MAX_LORE_FACT_BYTES, MAX_LORE_POOL_ENTRIES, MAX_LORE_POOLS,
    MAX_LORE_SOURCE_BYTES, MAX_LORE_SUBJECT_BYTES, MAX_LORE_TAG_BYTES, MAX_NPC_LORE_ENTRIES,
    MAX_NPC_LORE_TAGS, MAX_RESOLVED_LORE_BYTES, MAX_RESOLVED_LORE_ENTRIES, NpcKnowledge, Secrecy,
    familiarity_of, pool_key, validate_lore_tag,
};
pub use memory::{
    BUFFER_CAP, LongTermStore, MAX_MEMORY_BUFFER_BYTES, MAX_MEMORY_EVENT_CHARS,
    MAX_MEMORY_NOTE_BYTES, MAX_MEMORY_NOTES_PER_SUBJECT, MAX_MEMORY_RECALL_RECORDS,
    MAX_MEMORY_RECORD_BYTES, MAX_MEMORY_SUBJECT_BYTES, MAX_MEMORY_SUBJECTS_PER_NPC,
    MAX_MEMORY_SUMMARY_CHARS, MemoryBuffer, MemoryContext, MemoryEvent, MemoryKind, MemoryRecall,
    MemoryRecord, MemoryRecordError, MemoryUpdate, record_key,
};
pub use module::ability_submit_key;
pub use module::{
    MAX_TURN_DESCRIPTION_BYTES, MAX_TURN_EXITS, MAX_TURN_NAME_BYTES, MAX_TURN_PERSONA_BYTES,
    MAX_TURN_PRESENT, NpcModule, scene_turn_context,
};
pub use module::{public_faction_for, record_completed_work_relationship, set_public_faction};
pub use mush_ruleset::ProjectedAbility;
pub use profile::{
    AgentProfile, AgentProfileValidationError, KnowledgeBounds, MAX_AGENT_PROFILE_BYTES,
    MAX_PROFILE_BACKGROUND_BYTES, MAX_PROFILE_COMMAND_REF_BYTES, MAX_PROFILE_COMMAND_REFS,
    MAX_PROFILE_FALLBACK_BYTES, MAX_PROFILE_FALLBACK_LINES, MAX_PROFILE_GOALS,
    MAX_PROFILE_INTERFACES, MAX_PROFILE_KNOWLEDGE_FENCES, MAX_PROFILE_LIST_ENTRY_BYTES,
    MAX_PROFILE_PERSONA_BYTES, MAX_PROFILE_SPEECH_BYTES, MAX_PROFILE_WATCH_BYTES,
    MAX_PROFILE_WATCH_ENTRIES, NS, NpcTier, PROFILE_ATTR, SUMMARY_ATTR, THINKING_FEED,
};
pub use prompt::{ENGINE_FRAME, system_prompt, user_prompt};
pub use query::IntelListQuery;
pub use relationships::{
    PersonalRelation, PublicStanding, RelationshipEffect, RelationshipRecord, RelationshipSource,
    RelationshipStore, relationship_key,
};
pub use runtime::DEFAULT_SCENE_TURN_DEADLINE_SECS;
pub use runtime::{
    InterfaceAuditRow, MAX_INTERFACE_AUDIT_ROWS, MAX_PENDING_STIMULI, MAX_PENDING_STIMULUS_BYTES,
    NpcConfig, NpcHostState, NpcRuntime,
};
pub use snapshot::{
    AgentGrounding, AppliedAction, CommandGuidance, InterfaceToolSpec, MAX_AGENT_CAPABILITY_BYTES,
    MAX_AGENT_CAPABILITY_STATS_BYTES, MAX_AGENT_CAPABILITY_SUMMARY_BYTES,
    MAX_AGENT_CAPABILITY_TAG_BYTES, MAX_AGENT_CAPABILITY_TAGS, MAX_AGENT_CAPABILITY_TOOLS,
    MAX_AGENT_TOOL_DESCRIPTION_BYTES, MAX_AGENT_TOOL_NAME_BYTES, MAX_AGENT_TOOL_SCHEMA_BYTES,
    MAX_PRESENT_DESCRIPTION_BYTES, MAX_SCENE_DESCRIPTION_BYTES, MAX_SCENE_EXITS, MAX_SCENE_PRESENT,
    MAX_SCENE_TIME_BYTES, MAX_SCENE_WEATHER_BYTES, MAX_SNAPSHOT_NAME_BYTES,
    MAX_TURN_SNAPSHOT_BYTES, NoGrounding, PresentPlayer, RecalledMemory, SceneView,
    SnapshotValidationError, StimulusLine, TurnKind, TurnOutcome, TurnSnapshot,
    validate_agent_capability_profile,
};
pub use stimulus::{SpeechKind, Stimulus};
pub use turn::run_npc_turn_async;

/// File a validated observation through the knowledge owner. Producers must
/// establish the observer's authority and provenance before calling this seam.
pub fn file_observation(ctx: &mut mush_core::Ctx<'_>, record: IntelRecord) -> Result<(), String> {
    record.validate().map_err(|e| e.to_string())?;
    let principal = record.principal;
    let host = ctx
        .w
        .module_state::<NpcHostState>()
        .ok_or("Knowledge is unavailable.")?;
    let mut intel = host.intel.clone();
    let rows = intel.file(record).map_err(|e| e.to_string())?;
    let value = serde_json::to_value(rows).map_err(|e| e.to_string())?;
    let host = ctx
        .w
        .module_state_mut::<NpcHostState>()
        .ok_or("Knowledge is unavailable.")?;
    host.intel = intel;
    ctx.persist(mush_core::ModuleOp {
        ns: "npc".into(),
        op: mush_core::ModuleWrite::JsonSet {
            key: intel_key(principal),
            value,
        },
    });
    Ok(())
}
