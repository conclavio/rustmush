//! The mission scope a hired human NPC crew member carries (`FO-15`, `Q-R5`, `Q-F1`); the
//! live crew contracts are `tacsim::detachment::crew`'s.

use serde::{Deserialize, Serialize};

/// Authorized mission scope for a hired human NPC crew member (`FO-15`, `Q-R5`, `Q-F1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NpcMissionScope {
    /// Standard vehicle crew/operator seat under a player or NPC detachment commander.
    SubordinateOperator,
    /// Authorized to command a detachment sortie (`FO-15`, `Q-F1`, `Q-F2`).
    DetachmentCommander,
}
