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
/// `doctrine`, a draft seal is `draft`, and a filed seal reads `active`. The arena has, for a
/// filed seal, the licence's id and its filed certificate (revision, rating, declared and
/// minimum class) — no term, so no phase and none of the three flags. It seals filed minds on
/// two paths: an IC claim, whose admission held the term Active or Grace, and the OOC sim,
/// which reads no IC holding at all (D-UM-49) — so `active` there is the origin's word, not a
/// reading of any term. Passing the holding's own state to the arena's seal is cut M8e's.
#[must_use]
pub fn origin_state(origin: &crate::arena::profile_seal::MindOrigin) -> SealLicenceState {
    use crate::arena::profile_seal::MindOrigin;
    match origin {
        MindOrigin::Doctrine { .. } => SealLicenceState::Doctrine,
        MindOrigin::Draft { .. } => SealLicenceState::Draft,
        // No term in hand (see above); M8e passes the holding's state here.
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

    /// **The whole table** (the M8a fix round's F10): every phase under every one of the
    /// eight combinations of the three flags, against the precedence written out as data.
    #[test]
    fn m8a_licence_state_of_the_whole_table() {
        let phases = [
            (TermPhase::NotEffective, SealLicenceState::Pending),
            (TermPhase::Active, SealLicenceState::Active),
            (TermPhase::Grace, SealLicenceState::Grace),
            (TermPhase::Lapsed, SealLicenceState::Lapsed),
        ];
        let mut rows = 0_u32;
        for (phase, alone) in phases {
            for bits in 0_u8..8 {
                let (revoked, recalled, pending) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
                let expected = if recalled {
                    SealLicenceState::Recalled
                } else if revoked {
                    SealLicenceState::Revoked
                } else if pending {
                    SealLicenceState::Pending
                } else {
                    alone
                };
                assert_eq!(
                    licence_state_of(phase, revoked, recalled, pending),
                    expected,
                    "{phase:?} revoked={revoked} recalled={recalled} pending={pending}"
                );
                rows += 1;
            }
        }
        assert_eq!(rows, 32);
    }

    /// **`origin_state`'s three arms** (F10): doctrine, draft, and a filed seal reading
    /// `active` — the origin's word, since the arena holds no term for it.
    #[test]
    fn m8a_origin_state_of_each_origin() {
        use crate::arena::profile_seal::{FiledCertificate, MindOrigin};
        use mush_core::mechanics::DefinitionDigest;
        let digest = |b: u8| DefinitionDigest::from_bytes([b; 32]);
        assert_eq!(
            origin_state(&MindOrigin::Doctrine { revision: 1 }),
            SealLicenceState::Doctrine
        );
        assert_eq!(
            origin_state(&MindOrigin::Draft {
                character: mush_core::ObjId(7),
                mind: 1,
                revision: 2,
                digest: digest(1),
            }),
            SealLicenceState::Draft
        );
        let certificate = FiledCertificate {
            revision_character: mush_core::ObjId(77),
            revision_mind: 1,
            revision_number: 1,
            revision_digest: digest(1),
            source_digest: digest(9),
            rating_job: mush_core::CommitToken::from_bytes([4; 16]),
            rating_id: digest(2),
            rating_schema: 1,
            rating_digest: digest(3),
            compatibility: digest(4),
            definition: digest(5),
            declared: crate::arena::core::CoreClass::I,
            minimum: crate::arena::core::CoreClass::II,
        };
        assert_eq!(
            origin_state(&MindOrigin::Filed {
                charter: "1".into(),
                line: "1".into(),
                revision: 1,
                licence: "1".into(),
                certificate: Box::new(certificate),
            }),
            SealLicenceState::Active
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
