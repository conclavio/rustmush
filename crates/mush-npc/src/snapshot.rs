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

use std::fmt;

use mush_core::{ObjId, Outbox, World};
use mush_ruleset::{AbilityError, AbilityOutcome, AgentCapabilityProfile};
use serde::{Deserialize, Serialize};

use crate::action::NpcAction;
use crate::knowledge::{Familiarity, LoreEntry};
use crate::profile::AgentProfile;

/// Maximum encoded bytes in one complete legacy NPC turn snapshot.
///
/// K7 replaces this opaque payload with smaller projected prompt and private
/// decision records. Until then this ceiling prevents a crowded or
/// misconfigured world from creating an unbounded async handoff.
pub const MAX_TURN_SNAPSHOT_BYTES: usize = 1024 * 1024;
/// Maximum other players represented in one scene snapshot.
pub const MAX_SCENE_PRESENT: usize = 128;
/// Maximum exits represented in one scene snapshot.
pub const MAX_SCENE_EXITS: usize = 128;
/// Maximum bytes in an object or room name copied into a snapshot.
pub const MAX_SNAPSHOT_NAME_BYTES: usize = 256;
/// Maximum bytes in the room description copied into a snapshot.
pub const MAX_SCENE_DESCRIPTION_BYTES: usize = 16 * 1024;
/// Maximum bytes in one present player's short description.
pub const MAX_PRESENT_DESCRIPTION_BYTES: usize = 2 * 1024;
/// Maximum bytes in a weather digest copied into a snapshot.
pub const MAX_SCENE_WEATHER_BYTES: usize = 4 * 1024;
/// Maximum bytes in the rendered IC time copied into a snapshot.
pub const MAX_SCENE_TIME_BYTES: usize = 256;
/// Maximum tools advertised by one ruleset capability profile.
pub const MAX_AGENT_CAPABILITY_TOOLS: usize = 32;
/// Maximum capability tags advertised by one ruleset profile.
pub const MAX_AGENT_CAPABILITY_TAGS: usize = 128;
/// Maximum bytes in the prose summary of one ruleset capability profile.
pub const MAX_AGENT_CAPABILITY_SUMMARY_BYTES: usize = 32 * 1024;
/// Maximum encoded bytes in the stats value of one ruleset capability profile.
pub const MAX_AGENT_CAPABILITY_STATS_BYTES: usize = 256 * 1024;
/// Maximum bytes in one ruleset capability tag.
pub const MAX_AGENT_CAPABILITY_TAG_BYTES: usize = 256;
/// Maximum bytes in one ruleset ability/tool name.
pub const MAX_AGENT_TOOL_NAME_BYTES: usize = 128;
/// Maximum bytes in one ruleset ability/tool description.
pub const MAX_AGENT_TOOL_DESCRIPTION_BYTES: usize = 8 * 1024;
/// Maximum encoded bytes in one ruleset ability/tool parameter schema.
pub const MAX_AGENT_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
/// Maximum encoded bytes in one complete ruleset capability profile.
pub const MAX_AGENT_CAPABILITY_BYTES: usize = 512 * 1024;

/// Why a legacy NPC turn snapshot or its ruleset grounding was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotValidationError {
    TooManyPresent,
    TooManyExits,
    SceneFieldTooLarge,
    TooManyCapabilityTags,
    CapabilityTagTooLarge,
    TooManyCapabilityTools,
    CapabilitySummaryTooLarge,
    CapabilityStatsTooLarge,
    CapabilityToolTooLarge,
    CapabilityProfileTooLarge,
    SnapshotTooLarge,
    InvalidCommandGuidance,
    InvalidMemoryReferences,
}

impl fmt::Display for SnapshotValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyPresent => "NPC scene exceeds its participant limit",
            Self::TooManyExits => "NPC scene exceeds its exit limit",
            Self::SceneFieldTooLarge => "NPC scene contains an oversized prompt field",
            Self::TooManyCapabilityTags => "ruleset capability profile has too many tags",
            Self::CapabilityTagTooLarge => "ruleset capability profile has an oversized tag",
            Self::TooManyCapabilityTools => "ruleset capability profile has too many tools",
            Self::CapabilitySummaryTooLarge => "ruleset capability summary exceeds its byte limit",
            Self::CapabilityStatsTooLarge => "ruleset capability stats exceed their byte limit",
            Self::CapabilityToolTooLarge => "ruleset capability tool exceeds its byte limit",
            Self::CapabilityProfileTooLarge => {
                "encoded ruleset capability profile exceeds its byte limit"
            }
            Self::SnapshotTooLarge => "encoded NPC turn snapshot exceeds its byte limit",
            Self::InvalidCommandGuidance => {
                "NPC command guidance exceeds its bounds or contains an invalid command name"
            }
            Self::InvalidMemoryReferences => {
                "NPC recalled memory contains invalid reference metadata"
            }
        })
    }
}

impl std::error::Error for SnapshotValidationError {}

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
    /// How well the NPC knows this player, derived from episodic memory
    /// (design: M10d). Volatile per-asker trust — drives whether a
    /// [`Secrecy::Guarded`](crate::knowledge::Secrecy::Guarded) fact may be
    /// shared. Set on the world loop at snapshot time.
    #[serde(default)]
    pub familiarity: Familiarity,
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
    /// Engine-issued experience IDs actually shown in these notes.
    #[serde(default)]
    pub evidence_ids: Vec<u64>,
    /// Current belief IDs actually shown in these notes, eligible for revision.
    #[serde(default)]
    pub belief_ids: Vec<u64>,
}

/// The bounded, typed reference set captured with one dialogue prompt. It is
/// never reconstructed from model text or from memories that arrive later.
#[derive(Clone, Debug, Default)]
pub(crate) struct MemoryReferences {
    evidence: Vec<u64>,
    beliefs: Vec<u64>,
}

impl MemoryReferences {
    pub(crate) fn from_recalled(records: &[RecalledMemory]) -> Option<Self> {
        let limit = crate::experience::MAX_PERSONAL_RECALL_ENTRIES;
        let mut references = Self::default();
        for record in records {
            for (ids, captured) in [
                (&record.evidence_ids, &mut references.evidence),
                (&record.belief_ids, &mut references.beliefs),
            ] {
                if ids.len() > limit {
                    return None;
                }
                for &id in ids {
                    if id == 0 {
                        return None;
                    }
                    if !captured.contains(&id) {
                        if captured.len() >= limit {
                            return None;
                        }
                        captured.push(id);
                    }
                }
            }
        }
        Some(references)
    }

    pub(crate) fn allows(&self, evidence: Option<u64>, replaces: Option<u64>) -> bool {
        evidence.is_none_or(|id| self.evidence.contains(&id))
            && replaces.is_none_or(|id| self.beliefs.contains(&id))
    }
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
    /// The resolved, **shareable** world knowledge for this turn (per-NPC lore
    /// UNION shared-pool entries; design: M10d). Load-bearing invariant: this
    /// holds only [`Secrecy::Public`](crate::knowledge::Secrecy::Public) and
    /// [`Secrecy::Guarded`](crate::knowledge::Secrecy::Guarded) entries — a
    /// [`Secrecy::Secret`](crate::knowledge::Secrecy::Secret) fact is filtered
    /// out at the builder so it **never** enters a [`TurnSnapshot`] (nor its
    /// serialized JSON), let alone a prompt.
    #[serde(default)]
    pub knowledge: Vec<LoreEntry>,
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
    /// S6 (doc 14 §4.6): the interface abilities this NPC fronts, merged
    /// into the tool set beside the ruleset's; each runs as the asker.
    #[serde(default)]
    pub interfaces: Vec<InterfaceToolSpec>,
    /// Bounded registered help for this NPC's role, visible to the current
    /// asker. The player executes these commands; these are not NPC tools.
    #[serde(default)]
    pub command_guidance: Vec<CommandGuidance>,
}

/// One current, asker-visible command reference, resolved on the world loop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandGuidance {
    /// Canonical registered command name.
    pub command: String,
    /// Complete bounded help text, including any conditions and syntax.
    pub text: String,
}

/// One interface ability on a snapshot: which provider fronts it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InterfaceToolSpec {
    /// The provider id the profile named.
    pub provider: smol_str::SmolStr,
    /// The ability.
    pub ability: mush_core::InterfaceAbility,
}

impl TurnSnapshot {
    /// Validate the variable-size scene and ruleset portions plus the complete
    /// encoded async payload before it leaves the world task.
    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        validate_scene(&self.scene)?;
        validate_command_guidance(&self.command_guidance)?;
        MemoryReferences::from_recalled(&self.recalled)
            .ok_or(SnapshotValidationError::InvalidMemoryReferences)?;
        if let Some(profile) = &self.agent_profile {
            validate_agent_capability_profile(profile)?;
        }
        let encoded =
            serde_json::to_vec(self).map_err(|_| SnapshotValidationError::SnapshotTooLarge)?;
        if encoded.len() > MAX_TURN_SNAPSHOT_BYTES {
            return Err(SnapshotValidationError::SnapshotTooLarge);
        }
        Ok(())
    }
}

pub(crate) fn validate_command_guidance(
    cards: &[CommandGuidance],
) -> Result<(), SnapshotValidationError> {
    use crate::interface::{
        MAX_COMMAND_GUIDANCE, MAX_COMMAND_GUIDANCE_BYTES, MAX_COMMAND_GUIDANCE_TOTAL_BYTES,
    };
    if cards.len() > MAX_COMMAND_GUIDANCE {
        return Err(SnapshotValidationError::InvalidCommandGuidance);
    }
    let mut total = 0usize;
    for card in cards {
        let bytes = card.command.len().saturating_add(card.text.len());
        if !crate::profile::valid_command_ref(&card.command)
            || card.text.trim().is_empty()
            || bytes > MAX_COMMAND_GUIDANCE_BYTES
        {
            return Err(SnapshotValidationError::InvalidCommandGuidance);
        }
        total = total.saturating_add(bytes);
    }
    if total > MAX_COMMAND_GUIDANCE_TOTAL_BYTES {
        return Err(SnapshotValidationError::InvalidCommandGuidance);
    }
    Ok(())
}

/// Validate ruleset-provided prompt grounding before it is copied across the
/// async boundary. Ruleset code is trusted, but a plugin bug must not create
/// unbounded tool manifests or JSON schemas.
pub fn validate_agent_capability_profile(
    profile: &AgentCapabilityProfile,
) -> Result<(), SnapshotValidationError> {
    if profile.summary.len() > MAX_AGENT_CAPABILITY_SUMMARY_BYTES {
        return Err(SnapshotValidationError::CapabilitySummaryTooLarge);
    }
    if encoded_len(&profile.stats) > MAX_AGENT_CAPABILITY_STATS_BYTES {
        return Err(SnapshotValidationError::CapabilityStatsTooLarge);
    }
    if profile.tags.len() > MAX_AGENT_CAPABILITY_TAGS {
        return Err(SnapshotValidationError::TooManyCapabilityTags);
    }
    if profile
        .tags
        .iter()
        .any(|tag| tag.len() > MAX_AGENT_CAPABILITY_TAG_BYTES)
    {
        return Err(SnapshotValidationError::CapabilityTagTooLarge);
    }
    if profile.tools.len() > MAX_AGENT_CAPABILITY_TOOLS {
        return Err(SnapshotValidationError::TooManyCapabilityTools);
    }
    if profile.tools.iter().any(|tool| {
        tool.name.len() > MAX_AGENT_TOOL_NAME_BYTES
            || tool.description.len() > MAX_AGENT_TOOL_DESCRIPTION_BYTES
            || encoded_len(&tool.params_schema) > MAX_AGENT_TOOL_SCHEMA_BYTES
    }) {
        return Err(SnapshotValidationError::CapabilityToolTooLarge);
    }
    if encoded_len(profile) > MAX_AGENT_CAPABILITY_BYTES {
        return Err(SnapshotValidationError::CapabilityProfileTooLarge);
    }
    Ok(())
}

fn validate_scene(scene: &SceneView) -> Result<(), SnapshotValidationError> {
    if scene.present.len() > MAX_SCENE_PRESENT {
        return Err(SnapshotValidationError::TooManyPresent);
    }
    if scene.exits.len() > MAX_SCENE_EXITS {
        return Err(SnapshotValidationError::TooManyExits);
    }
    if scene.room_name.len() > MAX_SNAPSHOT_NAME_BYTES
        || scene.room_desc.len() > MAX_SCENE_DESCRIPTION_BYTES
        || scene.time.len() > MAX_SCENE_TIME_BYTES
        || scene
            .weather
            .as_ref()
            .is_some_and(|weather| weather.len() > MAX_SCENE_WEATHER_BYTES)
        || scene.present.iter().any(|player| {
            player.name.len() > MAX_SNAPSHOT_NAME_BYTES
                || player.short_desc.len() > MAX_PRESENT_DESCRIPTION_BYTES
        })
        || scene
            .exits
            .iter()
            .any(|exit| exit.len() > MAX_SNAPSHOT_NAME_BYTES)
    {
        return Err(SnapshotValidationError::SceneFieldTooLarge);
    }
    Ok(())
}

fn encoded_len(value: &impl Serialize) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |encoded| encoded.len())
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
        /// S6: the speaking player of the stimulus — the asker every
        /// interface ability runs as. `None` for an ambient turn.
        #[serde(default)]
        asker: Option<ObjId>,
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
    /// A completed rolling-summary fold. This does not create durable evidence
    /// or beliefs; model-generated summaries are never observations.
    Summary {
        /// The NPC.
        npc: ObjId,
        /// The new rolling summary.
        summary: String,
    },
    /// The memory fold failed; retain its source events for a later retry.
    #[serde(rename = "summary_failed")]
    SummaryFailed {
        /// The NPC.
        npc: ObjId,
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
            | TurnOutcome::SummaryFailed { npc }
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

    /// H2c: the projected legal action `name` with `input` maps to for `npc`,
    /// if the grounding's ruleset offers one (see
    /// [`Ruleset::projected_ability`](mush_ruleset::Ruleset::projected_ability)).
    /// The default maps nothing, so every ability stays on
    /// [`Self::invoke_ability`].
    fn projected_ability(
        &self,
        w: &World,
        npc: ObjId,
        name: &str,
        input: &serde_json::Value,
    ) -> Option<mush_ruleset::ProjectedAbility> {
        let _ = (w, npc, name, input);
        None
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

#[cfg(test)]
mod tests {
    use mush_ruleset::AgentToolSpec;
    use serde_json::json;
    use smol_str::SmolStr;

    use super::*;

    fn snapshot() -> TurnSnapshot {
        TurnSnapshot {
            kind: TurnKind::Dialogue,
            npc: ObjId(9),
            npc_name: "bounded npc".to_string(),
            profile: AgentProfile::default(),
            agent_profile: None,
            scene: SceneView {
                room_name: "room".to_string(),
                room_desc: String::new(),
                present: Vec::new(),
                time: "now".to_string(),
                weather: None,
                exits: Vec::new(),
            },
            knowledge: Vec::new(),
            summary: String::new(),
            recalled: Vec::new(),
            transcript: String::new(),
            stimuli: Vec::new(),
            actor: None,
            fold_events: Vec::new(),
            seed: 1,
            interfaces: Vec::new(),
            command_guidance: Vec::new(),
        }
    }

    fn tool() -> AgentToolSpec {
        AgentToolSpec {
            name: SmolStr::new_static("roll"),
            description: "roll a bounded pool".to_string(),
            params_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn capability_profile_rejects_unbounded_tools_and_schemas() {
        let mut profile = AgentCapabilityProfile {
            summary: String::new(),
            stats: json!({}),
            tags: Vec::new(),
            tools: vec![tool(); MAX_AGENT_CAPABILITY_TOOLS + 1],
        };
        assert_eq!(
            validate_agent_capability_profile(&profile),
            Err(SnapshotValidationError::TooManyCapabilityTools)
        );

        profile.tools = vec![tool()];
        profile.tools[0].params_schema =
            serde_json::Value::String("x".repeat(MAX_AGENT_TOOL_SCHEMA_BYTES + 1));
        assert_eq!(
            validate_agent_capability_profile(&profile),
            Err(SnapshotValidationError::CapabilityToolTooLarge)
        );
    }

    #[test]
    fn snapshot_rejects_unbounded_scene_and_complete_payload() {
        let mut snap = snapshot();
        snap.scene.present = (0..=MAX_SCENE_PRESENT)
            .map(|index| PresentPlayer {
                name: format!("player {index}"),
                short_desc: String::new(),
                familiarity: Familiarity::Stranger,
            })
            .collect();
        assert_eq!(
            snap.validate(),
            Err(SnapshotValidationError::TooManyPresent)
        );

        let mut snap = snapshot();
        snap.transcript = "x".repeat(MAX_TURN_SNAPSHOT_BYTES);
        assert_eq!(
            snap.validate(),
            Err(SnapshotValidationError::SnapshotTooLarge)
        );
    }

    #[test]
    fn older_snapshot_defaults_to_no_command_guidance() {
        let mut wire = serde_json::to_value(snapshot()).expect("snapshot JSON");
        wire.as_object_mut()
            .expect("object")
            .remove("command_guidance");
        let decoded: TurnSnapshot = serde_json::from_value(wire).expect("older snapshot");
        assert!(decoded.command_guidance.is_empty());
        assert!(decoded.validate().is_ok());
    }

    #[test]
    fn old_recollections_have_no_reference_authority_and_new_ids_are_bounded() {
        let old: RecalledMemory = serde_json::from_value(json!({
            "subject": "Alice", "notes": ["An old note containing experience #2"]
        }))
        .expect("legacy recalled memory");
        assert!(old.evidence_ids.is_empty());
        assert!(old.belief_ids.is_empty());
        let references =
            MemoryReferences::from_recalled(std::slice::from_ref(&old)).expect("legacy metadata");
        assert!(references.allows(None, None));
        assert!(!references.allows(Some(2), None));

        let mut snap = snapshot();
        snap.recalled = vec![old];
        snap.recalled[0].evidence_ids = vec![0];
        assert_eq!(
            snap.validate(),
            Err(SnapshotValidationError::InvalidMemoryReferences)
        );
        snap.recalled[0].evidence_ids = vec![1; crate::experience::MAX_PERSONAL_RECALL_ENTRIES + 1];
        assert_eq!(
            snap.validate(),
            Err(SnapshotValidationError::InvalidMemoryReferences)
        );
        snap.recalled[0].evidence_ids = vec![2];
        snap.recalled[0].belief_ids = vec![3];
        assert!(snap.validate().is_ok());
        let references = MemoryReferences::from_recalled(&snap.recalled).expect("valid metadata");
        assert!(references.allows(Some(2), Some(3)));
        assert!(
            !references.allows(Some(3), Some(2)),
            "evidence and belief namespaces stay distinct"
        );
    }

    #[test]
    fn snapshot_rejects_invalid_and_oversized_command_guidance() {
        use crate::interface::{
            MAX_COMMAND_GUIDANCE, MAX_COMMAND_GUIDANCE_BYTES, MAX_COMMAND_GUIDANCE_TOTAL_BYTES,
        };
        let mut snap = snapshot();
        let card = CommandGuidance {
            command: "+coop".into(),
            text: "+coop/enrol <name> signs the charter.".into(),
        };
        snap.command_guidance = vec![card.clone(); MAX_COMMAND_GUIDANCE];
        assert!(snap.validate().is_ok());
        snap.command_guidance.push(card.clone());
        assert_eq!(
            snap.validate(),
            Err(SnapshotValidationError::InvalidCommandGuidance)
        );

        snap.command_guidance = vec![card.clone()];
        snap.command_guidance[0].text = "x".repeat(MAX_COMMAND_GUIDANCE_BYTES - card.command.len());
        assert!(snap.validate().is_ok(), "per-card bound is inclusive");
        snap.command_guidance[0].text.push('x');
        assert_eq!(
            snap.validate(),
            Err(SnapshotValidationError::InvalidCommandGuidance)
        );

        snap.command_guidance = vec![
            CommandGuidance {
                command: "+coop".into(),
                text: "x".repeat(MAX_COMMAND_GUIDANCE_TOTAL_BYTES / 3),
            };
            3
        ];
        assert_eq!(
            snap.validate(),
            Err(SnapshotValidationError::InvalidCommandGuidance)
        );
        for command in ["+coop/enrol", "@Admin", "<system>"] {
            snap.command_guidance = vec![CommandGuidance {
                command: command.into(),
                ..card.clone()
            }];
            assert_eq!(
                snap.validate(),
                Err(SnapshotValidationError::InvalidCommandGuidance)
            );
        }
        snap.command_guidance = vec![CommandGuidance {
            text: " \n ".into(),
            ..card
        }];
        assert_eq!(
            snap.validate(),
            Err(SnapshotValidationError::InvalidCommandGuidance)
        );
    }
}
