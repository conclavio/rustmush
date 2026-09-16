//! The onboarding configuration (from `mushd.toml`'s `[onboarding]`): the two
//! start rooms and whether an unapproved character is held to chargen.
//!
//! Lived in the WoD ruleset until 16b cut R-a (R2) re-homed it: the composition
//! root and the host's config mapping need the type, and the game's own sheet
//! consumes it once the character rulesets are gone (cut R-b).

use crate::obj::{LIMBO, ObjId};

/// All-default (`chargen_required = false`, both rooms `#0`/Limbo) is a no-op:
/// the gate always passes, the hook never fires, the spawn move is a no-op, and
/// approval moves no one — exactly the pre-onboarding behaviour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnboardingConfig {
    /// When set, unapproved characters are gated to the chargen command set
    /// and dropped into the wizard on connect.
    pub chargen_required: bool,
    /// The live start room an approved character is moved to.
    pub start_room: ObjId,
    /// The restricted chargen area a fresh, unapproved character is placed in.
    pub start_room_unapproved: ObjId,
}

impl Default for OnboardingConfig {
    fn default() -> OnboardingConfig {
        OnboardingConfig {
            chargen_required: false,
            start_room: LIMBO,
            start_room_unapproved: LIMBO,
        }
    }
}
