//! **The licence state a seal is read under** (doc 17h §2.1, cut M8a; doc 17 §10, D-UM-42).
//!
//! A seal says which licence it was sealed under (`MindOrigin::licence_label`); it does not say
//! what state that licence is in, and it never will (M8a's A1): a `MindSeal` field would move
//! every digest over a seal — the claim book's `seal_digest`, the open epoch's, `audit.rs`'
//! check — for a fact that is the holding's, not the seal's. So the state is a **parameter**
//! of the line (`MindSeal::sealed_line`), derived here at the site that seals from the
//! holding's term phase and its flags, and the lines carry the word beside the licence label.
//!
//! The word itself ([`mush_roll::commerce::SealLicenceState`]) is defined in the Roll beside `TermPhase`, because the
//! Roll's inspection record (cut M8a's `@registry/inspect`) names it and the Roll cannot name
//! this crate; it is re-exported here, where the brief puts it, so tacsim code reads it from
//! one place.

pub use mush_roll::commerce::SealLicenceState;
use mush_roll::commerce::TermPhase;

/// **The state of a filed licence**, from its term's phase and the three flags that override
/// it, in the precedence recalled > revoked > pending > phase (doc 17h §2.1): a recalled
/// revision is recalled whatever its term says; a revoked term is revoked whatever its window
/// says; a pending transfer or a term that has not taken effect is `pending`; otherwise the
/// phase's own word.
#[must_use]
pub fn licence_state_of(
    phase: TermPhase,
    revoked: bool,
    recalled: bool,
    pending: bool,
) -> SealLicenceState {
    if recalled {
        return SealLicenceState::Recalled;
    }
    if revoked {
        return SealLicenceState::Revoked;
    }
    if pending {
        return SealLicenceState::Pending;
    }
    match phase {
        TermPhase::NotEffective => SealLicenceState::Pending,
        TermPhase::Active => SealLicenceState::Active,
        TermPhase::Grace => SealLicenceState::Grace,
        TermPhase::Lapsed => SealLicenceState::Lapsed,
    }
}

/// **The state a seal's origin alone can say** (M8a's A1 at the arena): a doctrine seal is
/// `doctrine`, a draft seal is `draft`, and a filed seal with no holding in hand reads `Active`
/// — the arena seals a filed mind only through an IC claim whose admission already held the
/// term Active or Grace, and the holding's own state at the arena's seal is cut M8e's to pass.
#[must_use]
pub fn origin_state(origin: &crate::arena::profile_seal::MindOrigin) -> SealLicenceState {
    use crate::arena::profile_seal::MindOrigin;
    match origin {
        MindOrigin::Doctrine { .. } => SealLicenceState::Doctrine,
        MindOrigin::Draft { .. } => SealLicenceState::Draft,
        // M8e passes the holding's phase here; until then the admission's word.
        MindOrigin::Filed { .. } => SealLicenceState::Active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every arm of the precedence** (doc 17h §2.1): recalled over revoked over pending over
    /// the phase, and each of the four phases alone.
    #[test]
    fn m8a_licence_state_of_every_arm() {
        for phase in [
            TermPhase::NotEffective,
            TermPhase::Active,
            TermPhase::Grace,
            TermPhase::Lapsed,
        ] {
            assert_eq!(
                licence_state_of(phase, true, true, true),
                SealLicenceState::Recalled,
                "{phase:?}: recalled wins"
            );
            assert_eq!(
                licence_state_of(phase, true, false, true),
                SealLicenceState::Revoked,
                "{phase:?}: revoked over pending"
            );
            assert_eq!(
                licence_state_of(phase, false, false, true),
                SealLicenceState::Pending,
                "{phase:?}: pending over the phase"
            );
        }
        assert_eq!(
            licence_state_of(TermPhase::NotEffective, false, false, false),
            SealLicenceState::Pending
        );
        assert_eq!(
            licence_state_of(TermPhase::Active, false, false, false),
            SealLicenceState::Active
        );
        assert_eq!(
            licence_state_of(TermPhase::Grace, false, false, false),
            SealLicenceState::Grace
        );
        assert_eq!(
            licence_state_of(TermPhase::Lapsed, false, false, false),
            SealLicenceState::Lapsed
        );
    }

    /// The wire words are snake_case and round-trip; `as_str` is the same word.
    #[test]
    fn m8a_the_state_words() {
        for (state, word) in [
            (SealLicenceState::Doctrine, "doctrine"),
            (SealLicenceState::Draft, "draft"),
            (SealLicenceState::Active, "active"),
            (SealLicenceState::Grace, "grace"),
            (SealLicenceState::Pending, "pending"),
            (SealLicenceState::Lapsed, "lapsed"),
            (SealLicenceState::Revoked, "revoked"),
            (SealLicenceState::Recalled, "recalled"),
        ] {
            assert_eq!(state.as_str(), word);
            assert_eq!(
                serde_json::to_value(state).expect("encodes"),
                serde_json::Value::String(word.to_owned())
            );
            assert_eq!(
                serde_json::from_value::<SealLicenceState>(serde_json::Value::String(
                    word.to_owned()
                ))
                .expect("decodes"),
                state
            );
        }
    }
}
