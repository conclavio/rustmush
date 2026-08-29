//! The owned data that crosses the async boundary (treaty ruling 3 — no world
//! references off-loop): the [`TurnSnapshot`] the world task assembles
//! synchronously and hands to the off-loop turn runner, and the [`TurnOutcome`]
//! that re-enters as the typed completion.
//!
//! Also the [`AgentGrounding`] seam: the driver backs it with the loaded
//! ruleset, and it does double duty — [`AgentGrounding::profile`] grounds the
//! NPC's sheet into the prompt on the world loop, and
//! [`AgentGrounding::invoke_ability`] executes a `UseAbility` (e.g. `wod_roll`)
//! **on the world loop during re-entry**, where it has the world + outbox the
//! ruleset's [`invoke_ability`](mush_ruleset::Ruleset::invoke_ability) needs to
//! read the NPC's own sheet, roll on the shared RNG, and emit to the room.

use mush_core::{ObjId, Outbox, World};
use mush_ruleset::{AbilityError, AbilityOutcome, AgentCapabilityProfile};
use serde::{Deserialize, Serialize};

use crate::action::NpcAction;
use crate::profile::AgentProfile;

/// What kind of off-loop turn this is.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnKind {
    /// A dialogue/action deliberation over the batched stimuli.
    Dialogue,
    /// A memory-fold: summarize the oldest half of the buffer.
    Summarize,
}

/// One present player, as named in the scene (name + short desc only — never
/// private data; design 03 §7 rule 2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PresentPlayer {
    /// Display name.
    pub name: String,
    /// One-line short description (empty if none).
    pub short_desc: String,
}

/// The public scene the NPC can perceive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneView {
    /// The room's name.
    pub room_name: String,
    /// The room's short description (public).
    pub room_desc: String,
    /// Present players (name + short desc).
    pub present: Vec<PresentPlayer>,
    /// The IC time string.
    pub time: String,
    /// A one-line weather digest, if any.
    #[serde(default)]
    pub weather: Option<String>,
    /// The room's existing exit names (so `move` targets are grounded).
    #[serde(default)]
    pub exits: Vec<String>,
}

/// One triggering stimulus, name-resolved for the prompt. Player text is kept
/// raw here and escaped at prompt-assembly time (design 03 §6.3 item 8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StimulusLine {
    /// The speaker/subject name (empty for system beats).
    pub who: String,
    /// The payload text (raw; escaped in the prompt).
    pub text: String,
    /// Whether it was addressed directly to the NPC.
    pub directed: bool,
    /// A short framing tag: `say` / `pose` / `arrived` / `left` / `poke` / ….
    pub tag: String,
}

/// One recalled durable memory (design 03 §6.5 layer 3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecalledMemory {
    /// Who/what it is about.
    pub subject: String,
    /// The remembered notes.
    pub notes: Vec<String>,
}

/// Everything the off-loop turn needs — owned data only. The world task builds
/// this synchronously while it has world access, then defers the LLM call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnSnapshot {
    /// Dialogue or summarize.
    pub kind: TurnKind,
    /// The NPC.
    pub npc: ObjId,
    /// The NPC's true name (rendered by the formatter — the NPC can't forge a
    /// different speaker).
    pub npc_name: String,
    /// The staff-authored profile.
    pub profile: AgentProfile,
    /// The ruleset grounding (summary/stats/tools), if the NPC has a sheet.
    #[serde(default)]
    pub agent_profile: Option<AgentCapabilityProfile>,
    /// The perceivable scene.
    pub scene: SceneView,
    /// The persisted rolling summary.
    #[serde(default)]
    pub summary: String,
    /// Long-term memories recalled by present-player subject.
    #[serde(default)]
    pub recalled: Vec<RecalledMemory>,
    /// The recent room transcript (token-capped).
    #[serde(default)]
    pub transcript: String,
    /// The batched triggering stimuli.
    pub stimuli: Vec<StimulusLine>,
    /// The triggering player, for LLM audit attribution (design 03 §4.3).
    /// `None` for ambient/summarize turns.
    #[serde(default)]
    pub actor: Option<ObjId>,
    /// For [`TurnKind::Summarize`]: the oldest-half events to fold in.
    #[serde(default)]
    pub fold_events: Vec<String>,
    /// A deterministic per-turn seed (the correlation id) for fallback picking
    /// and off-loop ability rolls.
    pub seed: u64,
}

/// One applied action, re-validated (and, for `UseAbility`, executed) in the
/// world task on re-entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppliedAction {
    /// The typed action.
    pub action: NpcAction,
}

/// The off-loop turn's result — re-enters as the module's typed completion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum TurnOutcome {
    /// Actions to apply (re-validated in the world task).
    Ok {
        /// The NPC.
        npc: ObjId,
        /// The applied actions (bounded, sanitized, ability-resolved).
        actions: Vec<AppliedAction>,
    },
    /// The NPC degraded to a fallback line (budget trip / refusal / disabled).
    /// The world task says the line through the normal formatter and fires a
    /// one-time staff-channel notice.
    Degraded {
        /// The NPC.
        npc: ObjId,
        /// The fallback line to say.
        line: String,
        /// Why it degraded (staff-facing).
        reason: String,
    },
    /// A completed memory fold: fold the summary + durable facts (a Summarize
    /// turn).
    Summary {
        /// The NPC.
        npc: ObjId,
        /// The new rolling summary.
        summary: String,
    },
    /// Nothing to do (do_nothing, empty, or a no-op) — clear in-flight only.
    Nothing {
        /// The NPC.
        npc: ObjId,
        /// A short reason for the audit trail.
        #[serde(default)]
        reason: String,
    },
}

impl TurnOutcome {
    /// The NPC this outcome is for (to clear in-flight / set cooldown).
    #[must_use]
    pub fn npc(&self) -> ObjId {
        match self {
            TurnOutcome::Ok { npc, .. }
            | TurnOutcome::Degraded { npc, .. }
            | TurnOutcome::Summary { npc, .. }
            | TurnOutcome::Nothing { npc, .. } => *npc,
        }
    }
}

/// The NPC↔ruleset seam (treaty ruling 6), running on the world loop. The
/// driver backs it with the loaded ruleset; a world with no ruleset grounding
/// uses [`NoGrounding`] and NPCs run dialogue-only (no sheet summary, no ability
/// tools). Two jobs:
///
/// - [`AgentGrounding::profile`] grounds the NPC's sheet into the prompt (the
///   snapshot phase, read-only) via
///   [`Ruleset::agent_profile`](mush_ruleset::Ruleset::agent_profile).
/// - [`AgentGrounding::invoke_ability`] executes a `UseAbility` **during
///   re-entry**, with the `&mut World` + `&mut Outbox` the ruleset's
///   [`invoke_ability`](mush_ruleset::Ruleset::invoke_ability) needs — it reads
///   the NPC's own sheet, rolls on the shared server RNG + roll log, and emits
///   the result to the room itself (the model only *requests* the roll; it can
///   never fabricate the outcome). Runs re-validated in the world task, so it
///   sees current state.
pub trait AgentGrounding: Send + Sync {
    /// The capability profile for `npc`, if it has a ruleset sheet.
    fn profile(&self, w: &World, npc: ObjId) -> Option<AgentCapabilityProfile>;

    /// Execute a validated NPC ability on the world loop. The default rejects
    /// every ability (dialogue-only worlds).
    ///
    /// # Errors
    ///
    /// Returns [`AbilityError`] when the ability is unknown or the input is
    /// invalid.
    fn invoke_ability(
        &self,
        w: &mut World,
        out: &mut Outbox,
        npc: ObjId,
        name: &str,
        input: serde_json::Value,
    ) -> Result<AbilityOutcome, AbilityError> {
        let _ = (w, out, npc, input);
        Err(AbilityError::Unknown(name.to_string()))
    }

    /// Give `npc` a fresh ruleset character sheet of `template` (so its
    /// abilities — `wod_roll` — have real traits to read), created by `by`
    /// through the ruleset's normal validated sheet path. Backs the
    /// `@agent/sheet` staff command. The default has no sheet ruleset, so it
    /// errs; the driver's adapter delegates to
    /// [`Ruleset`](mush_ruleset::Ruleset). Returns a short confirmation on
    /// success.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when no sheet ruleset is loaded or the
    /// template is unknown / the write fails.
    fn init_sheet(
        &self,
        w: &mut World,
        npc: ObjId,
        template: &str,
        by: ObjId,
    ) -> Result<String, String> {
        let _ = (w, npc, template, by);
        Err("no ruleset with character sheets is loaded on this server".to_string())
    }
}

/// The default grounding: no sheet, no abilities.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoGrounding;

impl AgentGrounding for NoGrounding {
    fn profile(&self, _w: &World, _npc: ObjId) -> Option<AgentCapabilityProfile> {
        None
    }
}
