//! **A unit's posture** (doc 45 unit AP.3; Q-45-3, Q-45-4, Q-45-23): the one standing word a
//! player leaves on a unit, read by whichever mind holds its controls.
//!
//! Every unit carries exactly one: [`Posture::Return`] unless its player set another. The
//! word is the same on the ground and in space and for the house default and a sealed mind
//! alike; what each host's mind does with it is that host's (AP.6 the field, AP.7 a rig on a
//! plan and a carrier, AP.9 space, AP.10 the simulator). This module is the word and nothing
//! a mind does with it.
//!
//! **Where it lives.** On the unit, never in a host: the record is [`record`], one row a
//! unit keyed by the unit's own object, kept with the game's durable state, so it is there
//! after a restart and while a field promotes and demotes around the unit. A host that flies
//! the unit copies the word into its own state for its minds to read
//! ([`crate::arena::state::UnitState::posture`]) and never becomes its keeper.

use serde::{Deserialize, Serialize};

pub mod order;
pub mod record;

/// **The posture a unit holds** (the module head). `Return` is every unit's default (Q-45-4).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Posture {
    /// Go home: the link-loss ruling. On the ground, back to its carrier; in space, on to the
    /// port it filed.
    #[default]
    Return,
    /// Stay where it was set and defend.
    Guard,
    /// Stay and hold its fire.
    Hold,
}

impl Posture {
    /// Every posture, in the order a list prints them.
    pub const ALL: [Posture; 3] = [Posture::Return, Posture::Guard, Posture::Hold];

    /// **The word a mind reads** (`View.posture`) and the cockpit keyframe carries
    /// (`posture`): `return`, `guard` or `hold`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Posture::Return => "return",
            Posture::Guard => "guard",
            Posture::Hold => "hold",
        }
    }

    /// **The word a player reads and types**: `RETURN`, `GUARD` or `HOLD`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Posture::Return => "RETURN",
            Posture::Guard => "GUARD",
            Posture::Hold => "HOLD",
        }
    }

    /// The posture `word` names, whatever its case, or `None`.
    #[must_use]
    pub fn parse(word: &str) -> Option<Posture> {
        Posture::ALL
            .into_iter()
            .find(|posture| posture.as_str().eq_ignore_ascii_case(word.trim()))
    }

    /// Whether this is the default, which no record, snapshot or hash spends a byte on.
    #[must_use]
    pub const fn is_return(&self) -> bool {
        matches!(self, Posture::Return)
    }

    /// **The byte the arena's state hash folds for a posture that is not the default**
    /// (`crate::arena::hash::state_hash`, item 1's sub-item 26). Never nought, and never
    /// changed: a recorded hash depends on it.
    #[must_use]
    pub const fn fold_byte(self) -> u8 {
        match self {
            Posture::Return => 0,
            Posture::Guard => 1,
            Posture::Hold => 2,
        }
    }
}

impl std::fmt::Display for Posture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::Posture;

    #[test]
    fn the_default_is_return_and_each_word_reads_back() {
        assert_eq!(Posture::default(), Posture::Return);
        assert!(Posture::Return.is_return());
        for posture in Posture::ALL {
            assert_eq!(Posture::parse(posture.as_str()), Some(posture));
            assert_eq!(Posture::parse(posture.label()), Some(posture));
            assert_eq!(
                serde_json::to_string(&posture).ok(),
                Some(format!("\"{}\"", posture.as_str())),
                "the stored word is the word a mind reads"
            );
            assert_eq!(posture.is_return(), posture == Posture::Return);
            assert_eq!(posture.fold_byte() == 0, posture.is_return());
        }
        assert_eq!(Posture::parse(" Guard "), Some(Posture::Guard));
        assert_eq!(Posture::parse("flee"), None);
        assert_eq!(Posture::parse(""), None);
    }
}
