//! The one error the party seams return (E6a, 08f §3.2).
//!
//! Hand-written [`Display`](std::fmt::Display) and [`std::error::Error`] rather than
//! a `thiserror` derive, so the dependency list stays `{mush-core, serde, serde_json,
//! smol_str}` and D-E6-5's "the party is not a principal and knows about nothing" is
//! a fact about `cargo tree`, not a comment.
//!
//! **These strings are not the player-facing text.** Every refusal a player reads for
//! a party they may not view is exactly [`crate::NO_SUCH_PARTY`] — identical for a
//! party that does not exist and one they simply cannot see — and the command layer
//! renders its own sentence for the rest. What is here is the *classification* the
//! ruleset switches on.

use std::fmt;

/// Why a party seam refused.
///
/// Renders through the crate's existence-hiding rule: a caller that turns one of
/// these into a line for a player must send [`crate::NO_SUCH_PARTY`] for
/// [`PartyError::NoStanding`], because "there is no such party" and "you may not see
/// that party" have to be one message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartyError {
    /// No such party — it never existed, or the caller may not see it. Never
    /// distinguish the two in anything a player reads.
    NoStanding,
    /// The party is in the wrong state for this transition: [`crate::lock`] on
    /// something that is not `Forming`, [`crate::release`] or [`crate::unlock`] on
    /// something that is not `Locked`, or any membership change once `Disbanded`.
    State,
    /// The party has no members. [`crate::lock`] refuses to freeze an empty roster
    /// against a contract.
    Empty,
    /// The roster is frozen against a contract, so this membership change is refused.
    /// A substitution needs the ruleset to amend the contract and call
    /// [`crate::unlock`] first (§3.2).
    Locked,
    /// A cap was reached; the payload names which one (`"party"`, `"invitations"`).
    TooLarge(&'static str),
}

impl PartyError {
    /// A short classification word, for a log line.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PartyError::NoStanding => "no standing",
            PartyError::State => "wrong state",
            PartyError::Empty => "empty",
            PartyError::Locked => "locked",
            PartyError::TooLarge(_) => "too large",
        }
    }
}

impl fmt::Display for PartyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PartyError::NoStanding => f.write_str("no such party"),
            PartyError::State => f.write_str("that party is not in a state for this"),
            PartyError::Empty => f.write_str("that party has no members"),
            PartyError::Locked => f.write_str("that party's roster is locked to a contract"),
            PartyError::TooLarge(what) => write!(f, "that party is already at its {what} limit"),
        }
    }
}

impl std::error::Error for PartyError {}

#[cfg(test)]
mod tests {
    use super::PartyError;

    #[test]
    fn every_variant_renders_and_labels() {
        for e in [
            PartyError::NoStanding,
            PartyError::State,
            PartyError::Empty,
            PartyError::Locked,
            PartyError::TooLarge("party"),
        ] {
            assert!(!e.to_string().is_empty());
            assert!(!e.label().is_empty());
        }
        let boxed: Box<dyn std::error::Error> = Box::new(PartyError::State);
        assert_eq!(boxed.to_string(), "that party is not in a state for this");
    }
}
