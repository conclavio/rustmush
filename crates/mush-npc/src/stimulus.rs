//! [`Stimulus`] — what wakes an NPC (design 03 §6.2). The module's world-event
//! observers translate [`WorldEvent`](mush_core::module::WorldEvent)s into
//! stimuli and enqueue them on the target NPC's runtime mailbox; the tick
//! coalesces a batch into one LLM turn.
//!
//! **No players, no stimuli.** The observers only enqueue when a connected
//! player shares the NPC's room, so an NPC alone in a room receives nothing and
//! burns zero tokens.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

/// The flavor of a room speech stimulus (mirrors
//  the subset of `NarrativeKind` an NPC reacts to).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeechKind {
    /// Plain `say`.
    Say,
    /// A `pose`/emote.
    Pose,
    /// A semipose.
    SemiPose,
    /// A room `@emit`.
    Emit,
}

/// One thing that happened near the NPC, awaiting deliberation.
///
/// Speech carries the speaker and text; arrivals/departures carry only who.
/// [`Stimulus::is_low_value`] marks the ones that are safe to drop when they
/// go stale mid-turn (arrivals/departures/ambient), so rapid churn never
/// forces a wasted turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Stimulus {
    /// Speech addressed to the NPC (its name appears in the line).
    SpokenTo {
        /// The speaker.
        by: ObjId,
        /// What they said (plain text; escaped at prompt-assembly time).
        text: String,
        /// The speech flavor.
        kind: SpeechKind,
    },
    /// Ambient room speech the NPC merely overheard.
    HeardInRoom {
        /// The speaker.
        by: ObjId,
        /// What they said.
        text: String,
    },
    /// A player entered the NPC's room.
    PlayerArrived {
        /// The arriver.
        who: ObjId,
    },
    /// A player left the NPC's room.
    PlayerLeft {
        /// The leaver.
        who: ObjId,
    },
    /// A scheduled ambient beat (only ever queued while players share the
    /// room).
    AmbientTimer,
    /// A digested ruleset/game event (combat, weather change, …).
    GameEvent {
        /// A short human-readable digest.
        digest: String,
    },
    /// A staff out-of-band prod (`@agent/poke`).
    StaffPoke {
        /// The staffer.
        by: ObjId,
        /// The note handed to the NPC.
        note: String,
    },
}

impl Stimulus {
    /// The subject actor this stimulus is about, if any (used to key memory
    /// recall on present players).
    #[must_use]
    pub fn subject(&self) -> Option<ObjId> {
        match self {
            Stimulus::SpokenTo { by, .. }
            | Stimulus::HeardInRoom { by, .. }
            | Stimulus::StaffPoke { by, .. } => Some(*by),
            Stimulus::PlayerArrived { who } | Stimulus::PlayerLeft { who } => Some(*who),
            Stimulus::AmbientTimer | Stimulus::GameEvent { .. } => None,
        }
    }

    /// Whether this stimulus is a direct address (shortens cooldown; never
    /// dropped as stale).
    #[must_use]
    pub fn is_direct(&self) -> bool {
        matches!(self, Stimulus::SpokenTo { .. } | Stimulus::StaffPoke { .. })
    }

    /// Whether this stimulus is low value — safe to drop when it goes stale
    /// while a turn is in flight (arrivals/departures/ambient beats). Speech
    /// is always kept.
    #[must_use]
    pub fn is_low_value(&self) -> bool {
        matches!(
            self,
            Stimulus::PlayerArrived { .. } | Stimulus::PlayerLeft { .. } | Stimulus::AmbientTimer
        )
    }
}
