//! [`AgentProfile`] — the staff-authored, per-NPC configuration stored as the
//! JSON attribute [`PROFILE_ATTR`] on an [`ObjKind::Npc`](mush_core::ObjKind)
//! object (design 03 §6.1), plus the [`NpcTier`] → route / cooldown / budget
//! mapping.
//!
//! The profile is the *only* NPC-specific data that ever enters a prompt
//! (design 03 §7 rule 2: no secrets in prompts). It carries no API keys, no
//! other players' data, and no staff notes beyond the NPC's own persona —
//! anything truly secret is kept out of the context entirely.

use std::fmt;
use std::time::Duration;

use mush_faction::Principal;
use mush_llm::{LlmFeature, NpcTier as LlmNpcTier};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

/// The module's persistence namespace and [`GameModule`](mush_core::GameModule)
/// identity.
pub const NS: &str = "npc";

/// The attribute an NPC's [`AgentProfile`] is stored under (JSON).
pub const PROFILE_ATTR: &str = "agent:profile";

/// The attribute the rolling conversation summary is persisted under
/// (design 03 §6.5 layer 2).
pub const SUMMARY_ATTR: &str = "agent:memory:summary";

/// The web panel/event feed a `npc.thinking` indicator rides on
/// (design 03 §8: web shows a subtle indicator; terminal users just see the
/// reply arrive).
pub const THINKING_FEED: &str = "npc.thinking";

/// Maximum bytes in the freeform persona field.
pub const MAX_PROFILE_PERSONA_BYTES: usize = 8 * 1024;
/// Maximum bytes in the freeform background field.
pub const MAX_PROFILE_BACKGROUND_BYTES: usize = 32 * 1024;
/// Maximum bytes in the speech-style field.
pub const MAX_PROFILE_SPEECH_BYTES: usize = 4 * 1024;
/// Maximum goals stored on one profile.
pub const MAX_PROFILE_GOALS: usize = 64;
/// Maximum knowledge-fence entries in either allow/deny list.
pub const MAX_PROFILE_KNOWLEDGE_FENCES: usize = 64;
/// Maximum fallback lines stored on one profile.
pub const MAX_PROFILE_FALLBACK_LINES: usize = 32;
/// Maximum informant watch entries stored on one profile.
pub const MAX_PROFILE_WATCH_ENTRIES: usize = 128;
/// Maximum bytes in one goal or knowledge-fence entry.
pub const MAX_PROFILE_LIST_ENTRY_BYTES: usize = 2 * 1024;
/// Maximum bytes in one fallback line.
pub const MAX_PROFILE_FALLBACK_BYTES: usize = 2 * 1024;
/// Maximum bytes in one watched subject.
pub const MAX_PROFILE_WATCH_BYTES: usize = 256;
/// Maximum encoded bytes in one complete profile.
pub const MAX_AGENT_PROFILE_BYTES: usize = 256 * 1024;

/// Invalid persisted or staff-authored agent profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentProfileValidationError {
    /// More than [`MAX_PROFILE_INTERFACES`] providers.
    TooManyInterfaces,
    /// An empty or over-long provider id.
    InterfaceTooLarge,
    PersonaTooLarge,
    BackgroundTooLarge,
    SpeechTooLarge,
    TooManyGoals,
    TooManyKnowledgeFences,
    TooManyFallbackLines,
    TooManyWatchEntries,
    EmptyListEntry,
    ListEntryTooLarge,
    FallbackTooLarge,
    WatchEntryTooLarge,
    DocumentTooLarge,
}

impl fmt::Display for AgentProfileValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            AgentProfileValidationError::TooManyInterfaces => "too many interfaces",
            AgentProfileValidationError::InterfaceTooLarge => "an interface id is 1 to 32 bytes",
            Self::PersonaTooLarge => "agent persona exceeds its byte limit",
            Self::BackgroundTooLarge => "agent background exceeds its byte limit",
            Self::SpeechTooLarge => "agent speech style exceeds its byte limit",
            Self::TooManyGoals => "agent goal count exceeds its hard limit",
            Self::TooManyKnowledgeFences => "agent knowledge-fence count exceeds its hard limit",
            Self::TooManyFallbackLines => "agent fallback-line count exceeds its hard limit",
            Self::TooManyWatchEntries => "agent watch-entry count exceeds its hard limit",
            Self::EmptyListEntry => "agent profile lists may not contain empty entries",
            Self::ListEntryTooLarge => "agent goal or knowledge fence exceeds its byte limit",
            Self::FallbackTooLarge => "agent fallback line exceeds its byte limit",
            Self::WatchEntryTooLarge => "agent watch entry exceeds its byte limit",
            Self::DocumentTooLarge => "encoded agent profile exceeds its byte limit",
        })
    }
}

impl std::error::Error for AgentProfileValidationError {}

/// NPC importance tier — selects the dialogue route, cooldown, and budget
/// defaults (design 03 §6.1). Mirrors [`mush_llm::NpcTier`] but lives here so
/// the profile serializes a stable, human-friendly name.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NpcTier {
    /// A named, plot-relevant NPC (strong model, short cooldown).
    Major,
    /// A recurring background NPC (cheap model).
    #[default]
    Minor,
    /// A throwaway NPC (cheapest model, long cooldown).
    Extra,
}

impl NpcTier {
    /// The `mush-llm` tier this maps to (drives the dialogue route).
    #[must_use]
    pub fn llm(self) -> LlmNpcTier {
        match self {
            NpcTier::Major => LlmNpcTier::Major,
            NpcTier::Minor => LlmNpcTier::Minor,
            NpcTier::Extra => LlmNpcTier::Extra,
        }
    }

    /// The dialogue feature route for this tier.
    #[must_use]
    pub fn dialogue_feature(self) -> LlmFeature {
        LlmFeature::NpcDialogue(self.llm())
    }

    /// The default minimum time between LLM turns (design 03 §6.2:
    /// Major 5s / Minor 15s / Extra 60s).
    #[must_use]
    pub fn default_cooldown(self) -> Duration {
        Duration::from_secs(match self {
            NpcTier::Major => 5,
            NpcTier::Minor => 15,
            NpcTier::Extra => 60,
        })
    }
}

/// Hard knowledge fences (design 03 §6.1). Phrased as "you know X" /
/// "you do not know Y"; anything *truly* secret is simply never placed in the
/// prompt (§7 rule 2), so these bound embarrassment, not security.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KnowledgeBounds {
    /// Facts the NPC knows.
    pub knows: Vec<String>,
    /// Facts the NPC must not claim to know.
    pub does_not_know: Vec<String>,
}

/// A staff-authored NPC agent profile (design 03 §6.1), stored as JSON under
/// [`PROFILE_ATTR`]. Deserialization is lenient (`#[serde(default)]`) so a
/// sparse or older document always loads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentProfile {
    /// Importance tier — route / cooldown / budget defaults.
    pub tier: NpcTier,
    /// Personality, mannerisms.
    pub persona: String,
    /// History; what shaped them.
    pub background: String,
    /// Register, dialect, verbosity.
    pub speech_style: String,
    /// Current motivations.
    pub goals: Vec<String>,
    /// Knowledge fences.
    pub knowledge: KnowledgeBounds,
    /// Minimum seconds between LLM turns (overrides the tier default when
    /// `Some` and non-zero).
    pub cooldown_s: Option<u32>,
    /// Per-NPC daily token budget override. When set, it **replaces** the
    /// config `per_npc_default` ceiling for this NPC: the off-loop turn carries
    /// it into the call's `CallCtx` (`with_npc_limit`), and the `LlmService`
    /// npc budget scope enforces it pre-call. Staff use it to cap a runaway
    /// agent (or raise a plot-critical one).
    pub daily_tokens: Option<u64>,
    /// Whether the agent is active. `false` (or `@llm/kill`) parks it: no
    /// wakes, no tokens.
    pub enabled: bool,
    /// Canned lines the NPC degrades to on a budget trip / provider refusal /
    /// disabled service (design 03 §6.6 — never an error to players).
    pub fallback_lines: Vec<String>,
    /// The M12 informant loop: who this NPC reports intel to — a character
    /// (domitor / employer; the M12 original, wire-compatible as a bare
    /// dbref) or a faction, whose clearance-gated pool members read via
    /// `+intel`. `None` (the default) means the NPC files nothing — the loop
    /// is entirely opt-in and additive.
    pub principal: Option<Principal>,
    /// The willingness gate. `true` (default): a loyal informant files what it
    /// perceives to its `principal`. `false`: a resentful asset that reports
    /// nothing, however much it sees.
    pub loyal: bool,
    /// Subjects of interest (lowercased names / topics). An event about a
    /// watched subject is reportable; with an empty list only inherently
    /// notable events (a pierced concealment) are filed, keeping the log
    /// signal-rich.
    pub watch: Vec<String>,
    /// S6 (doc 14 §4.6): the interface-ability providers this NPC fronts, by
    /// registered id (`atlas`, `weather`, `market`, `build`, `rules`,
    /// `travel`, `request`, `help`). Every ability runs as the asker.
    #[serde(default)]
    pub interfaces: Vec<SmolStr>,
}

/// The most providers one NPC fronts.
pub const MAX_PROFILE_INTERFACES: usize = 8;

/// The longest provider id.
pub const MAX_PROFILE_INTERFACE_BYTES: usize = 32;

impl Default for AgentProfile {
    fn default() -> AgentProfile {
        AgentProfile {
            tier: NpcTier::Minor,
            persona: String::new(),
            background: String::new(),
            speech_style: String::new(),
            goals: Vec::new(),
            knowledge: KnowledgeBounds::default(),
            cooldown_s: None,
            daily_tokens: None,
            enabled: true,
            fallback_lines: Vec::new(),
            principal: None,
            loyal: true,
            watch: Vec::new(),
            interfaces: Vec::new(),
        }
    }
}

impl AgentProfile {
    /// Validate all prompt-facing counts, fields, and the complete persisted
    /// document before this profile can be stored or used by an agent turn.
    pub fn validate(&self) -> Result<(), AgentProfileValidationError> {
        if self.persona.len() > MAX_PROFILE_PERSONA_BYTES {
            return Err(AgentProfileValidationError::PersonaTooLarge);
        }
        if self.interfaces.len() > MAX_PROFILE_INTERFACES {
            return Err(AgentProfileValidationError::TooManyInterfaces);
        }
        if self
            .interfaces
            .iter()
            .any(|i| i.is_empty() || i.len() > MAX_PROFILE_INTERFACE_BYTES)
        {
            return Err(AgentProfileValidationError::InterfaceTooLarge);
        }
        if self.background.len() > MAX_PROFILE_BACKGROUND_BYTES {
            return Err(AgentProfileValidationError::BackgroundTooLarge);
        }
        if self.speech_style.len() > MAX_PROFILE_SPEECH_BYTES {
            return Err(AgentProfileValidationError::SpeechTooLarge);
        }
        validate_list(
            &self.goals,
            MAX_PROFILE_GOALS,
            MAX_PROFILE_LIST_ENTRY_BYTES,
            AgentProfileValidationError::TooManyGoals,
            AgentProfileValidationError::ListEntryTooLarge,
        )?;
        validate_list(
            &self.knowledge.knows,
            MAX_PROFILE_KNOWLEDGE_FENCES,
            MAX_PROFILE_LIST_ENTRY_BYTES,
            AgentProfileValidationError::TooManyKnowledgeFences,
            AgentProfileValidationError::ListEntryTooLarge,
        )?;
        validate_list(
            &self.knowledge.does_not_know,
            MAX_PROFILE_KNOWLEDGE_FENCES,
            MAX_PROFILE_LIST_ENTRY_BYTES,
            AgentProfileValidationError::TooManyKnowledgeFences,
            AgentProfileValidationError::ListEntryTooLarge,
        )?;
        validate_list(
            &self.fallback_lines,
            MAX_PROFILE_FALLBACK_LINES,
            MAX_PROFILE_FALLBACK_BYTES,
            AgentProfileValidationError::TooManyFallbackLines,
            AgentProfileValidationError::FallbackTooLarge,
        )?;
        validate_list(
            &self.watch,
            MAX_PROFILE_WATCH_ENTRIES,
            MAX_PROFILE_WATCH_BYTES,
            AgentProfileValidationError::TooManyWatchEntries,
            AgentProfileValidationError::WatchEntryTooLarge,
        )?;
        let encoded =
            serde_json::to_vec(self).map_err(|_| AgentProfileValidationError::DocumentTooLarge)?;
        if encoded.len() > MAX_AGENT_PROFILE_BYTES {
            return Err(AgentProfileValidationError::DocumentTooLarge);
        }
        Ok(())
    }

    /// The effective cooldown: the explicit override if set and non-zero,
    /// else the tier default.
    #[must_use]
    pub fn cooldown(&self) -> Duration {
        match self.cooldown_s {
            Some(s) if s > 0 => Duration::from_secs(u64::from(s)),
            _ => self.tier.default_cooldown(),
        }
    }

    /// Pick a fallback line deterministically from `seed` (no RNG dependency;
    /// the seed is the turn correlation id). Returns a generic line when the
    /// profile lists none.
    #[must_use]
    pub fn fallback_line(&self, seed: u64) -> String {
        if self.fallback_lines.is_empty() {
            return "The NPC seems preoccupied and does not respond.".to_string();
        }
        let idx = (seed as usize) % self.fallback_lines.len();
        self.fallback_lines[idx].clone()
    }
}

fn validate_list(
    values: &[String],
    max_items: usize,
    max_entry_bytes: usize,
    count_error: AgentProfileValidationError,
    size_error: AgentProfileValidationError,
) -> Result<(), AgentProfileValidationError> {
    if values.len() > max_items {
        return Err(count_error);
    }
    for value in values {
        if value.trim().is_empty() {
            return Err(AgentProfileValidationError::EmptyListEntry);
        }
        if value.len() > max_entry_bytes {
            return Err(size_error);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_defaults_match_design() {
        assert_eq!(NpcTier::Major.default_cooldown(), Duration::from_secs(5));
        assert_eq!(NpcTier::Minor.default_cooldown(), Duration::from_secs(15));
        assert_eq!(NpcTier::Extra.default_cooldown(), Duration::from_secs(60));
    }

    #[test]
    fn cooldown_override_wins_when_nonzero() {
        let mut p = AgentProfile {
            tier: NpcTier::Extra,
            cooldown_s: Some(3),
            ..AgentProfile::default()
        };
        assert_eq!(p.cooldown(), Duration::from_secs(3));
        // Zero override falls back to the tier default.
        p.cooldown_s = Some(0);
        assert_eq!(p.cooldown(), Duration::from_secs(60));
        p.cooldown_s = None;
        assert_eq!(p.cooldown(), Duration::from_secs(60));
    }

    #[test]
    fn sparse_profile_deserializes_with_defaults() {
        let p: AgentProfile = serde_json::from_str(r#"{"persona":"a gruff barkeep"}"#).unwrap();
        assert_eq!(p.persona, "a gruff barkeep");
        assert_eq!(p.tier, NpcTier::Minor);
        assert!(p.enabled);
        assert!(p.goals.is_empty());
        // M12 informant fields default additively: no principal ⇒ files
        // nothing, loyal by default, empty watch list (P3).
        assert_eq!(p.principal, None);
        assert!(p.loyal);
        assert!(p.watch.is_empty());
    }

    #[test]
    fn informant_fields_round_trip_through_serde() {
        let p = AgentProfile {
            principal: Some(Principal::Character(mush_core::ObjId(42))),
            loyal: false,
            watch: vec!["alice".into(), "the docks".into()],
            ..AgentProfile::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: AgentProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.principal,
            Some(Principal::Character(mush_core::ObjId(42)))
        );
        assert!(!back.loyal);
        assert_eq!(
            back.watch,
            vec!["alice".to_string(), "the docks".to_string()]
        );
    }

    #[test]
    fn m12_era_profile_docs_still_load() {
        // Before M21 the persisted form was a bare dbref number; it must keep
        // deserializing as a character principal.
        let p: AgentProfile = serde_json::from_str(r#"{"principal": 42}"#).unwrap();
        assert_eq!(
            p.principal,
            Some(Principal::Character(mush_core::ObjId(42)))
        );
        // The faction form round-trips.
        let p: AgentProfile = serde_json::from_str(r#"{"principal": {"faction": 3}}"#).unwrap();
        assert_eq!(
            p.principal,
            Some(Principal::faction(mush_faction::FactionId(3)))
        );
    }

    #[test]
    fn fallback_line_is_deterministic_and_bounded() {
        let p = AgentProfile {
            fallback_lines: vec!["grunts.".into(), "shrugs.".into()],
            ..AgentProfile::default()
        };
        assert_eq!(p.fallback_line(0), "grunts.");
        assert_eq!(p.fallback_line(1), "shrugs.");
        assert_eq!(p.fallback_line(2), "grunts.");
        // Empty list yields a safe generic line, never a panic.
        let empty = AgentProfile::default();
        assert!(!empty.fallback_line(7).is_empty());
    }

    #[test]
    fn validation_rejects_prompt_growth_before_storage() {
        let oversized = AgentProfile {
            persona: "x".repeat(MAX_PROFILE_PERSONA_BYTES + 1),
            ..AgentProfile::default()
        };
        assert_eq!(
            oversized.validate(),
            Err(AgentProfileValidationError::PersonaTooLarge)
        );

        let too_many = AgentProfile {
            goals: (0..=MAX_PROFILE_GOALS)
                .map(|index| format!("goal {index}"))
                .collect(),
            ..AgentProfile::default()
        };
        assert_eq!(
            too_many.validate(),
            Err(AgentProfileValidationError::TooManyGoals)
        );
    }

    #[test]
    fn default_and_representative_profile_validate() {
        assert!(AgentProfile::default().validate().is_ok());
        let profile = AgentProfile {
            persona: "a careful fixer".into(),
            background: "keeps the neighborhood supplied".into(),
            goals: vec!["protect regulars".into()],
            knowledge: KnowledgeBounds {
                knows: vec!["local routes".into()],
                does_not_know: vec!["the hidden vault code".into()],
            },
            fallback_lines: vec!["Give me a minute.".into()],
            watch: vec!["alice".into()],
            ..AgentProfile::default()
        };
        assert!(profile.validate().is_ok());
    }
}
