//! Pure strict consent and arithmetic tests only; no owner authority claim.
use super::*;
use crate::{AccountId, Amount, InstrumentId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, ObjId};
use serde_json::json;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn action(id: &str, version: u32) -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("roll")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new(id)?,
            version,
            json!({"fixture":"inactive_pending_consent"}),
        )?,
    )?)
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}
pub(super) fn sample() -> Result<CharterPendingConsentV1> {
    let terms = CharterPendingTermsV1 {
        format: 1,
        offer: token(3),
        offer_action: action(OFFER_ACTION, 2)?,
        offer_digest: DefinitionDigest::from_bytes([255; 32]),
        charter: ObjId(8),
        birth: token(1),
        ownership: CharterOwnershipBasisV1 {
            generation: 1,
            operation: token(1),
            digest: DefinitionDigest::from_bytes([255; 32]),
        },
        owner_term: CharterTermBasisV1 {
            licence: 1,
            generation: 1,
            operation: token(1),
            digest: DefinitionDigest::from_bytes([255; 32]),
            term_end: 1000,
            grace_end: 2000,
        },
        title_revision: 1,
        seller: ObjId(1),
        buyer: ObjId(2),
        seller_account: AccountId(0),
        instrument: InstrumentId::new("cr").ok_or("instrument")?,
        price: Amount::from_minor(1),
        offered_at_ic: stamp(100),
        offered_at_utc: 100,
        expires_at_utc: 200,
        settle_seconds: 100,
    };
    let row = CharterPendingConsentV1 {
        format: 1,
        operation: token(4),
        effect: action(FUND_ACTION, 1)?,
        payer: AccountId(1),
        accepted_at_ic: stamp(110),
        accepted_at_utc: 110,
        deadline: terms.deadline(&stamp(110))?,
        terms,
    };
    row.validate()?;
    Ok(row)
}
#[test]
fn explicit_pending_consent_rejects_old_or_crossed_actions_and_identity() -> Result {
    let good = sample()?;
    for n in 0..12 {
        let mut row = good.clone();
        match n {
            0 => row.terms.offer_action = action(OFFER_ACTION, 1)?,
            1 => row.effect = action("accept_mind_charter_transfer", 1)?,
            2 => row.effect = action(FUND_ACTION, 2)?,
            3 => row.operation = row.terms.offer,
            4 => row.terms.seller = row.terms.buyer,
            5 => row.payer = row.terms.seller_account,
            6 => row.terms.price = Amount::from_minor(0),
            7 => row.terms.ownership.generation = 65537,
            8 => row.terms.ownership.operation = token(9),
            9 => row.terms.title_revision = u64::MAX,
            10 => row.terms.owner_term.generation = 0,
            _ => row.terms.owner_term.grace_end = row.terms.owner_term.term_end - 1,
        }
        assert!(row.validate().is_err());
        assert!(
            serde_json::from_value::<CharterPendingConsentV1>(serde_json::to_value(row)?).is_err()
        );
    }
    Ok(())
}
#[test]
fn deadline_is_positive_and_term_cap_may_already_be_due() -> Result {
    let mut row = sample()?;
    assert!(!row.deadline_reached(&stamp(209))?);
    assert!(row.deadline_reached(&stamp(210))?);
    assert!(row.deadline_reached(&stamp(300))?);
    row.terms.owner_term.term_end = 105;
    row.deadline = row.terms.deadline(&row.accepted_at_ic)?;
    assert_eq!(row.deadline.at, 105);
    assert!(row.deadline_reached(&row.accepted_at_ic)?);
    assert!(row.deadline_reached(&stamp(109)).is_err());
    let mut changed = stamp(210);
    changed.clock = ClockRef::WorldIc { generation: 2 };
    assert!(row.deadline_reached(&changed).is_err());
    // A different clock cannot be passed to this settlement predicate. A real
    // separate owner-proved ClockChanged refund must handle it later.
    Ok(())
}
#[test]
fn exact_deadline_and_offer_clocks_are_not_caller_extension_fields() -> Result {
    let good = sample()?;
    for n in 0..8 {
        let mut row = good.clone();
        match n {
            0 => row.deadline.at += 1,
            1 => row.deadline.clock = ClockRef::WorldIc { generation: 2 },
            2 => row.accepted_at_ic.at = 99,
            3 => row.accepted_at_ic.clock = ClockRef::WorldIc { generation: 2 },
            4 => row.accepted_at_utc = 99,
            5 => row.accepted_at_utc = 200,
            6 => row.terms.settle_seconds = 0,
            _ => row.terms.settle_seconds = 259201,
        }
        assert!(row.validate().is_err());
    }
    let mut at = good;
    at.accepted_at_utc = at.terms.expires_at_utc - 1;
    at.validate()?;
    Ok(())
}
#[test]
fn bounded_strict_decoder_retains_explicit_fields_and_canonical_bytes() -> Result {
    let good = sample()?;
    let value = serde_json::to_value(&good)?;
    let raw =
        mush_core::canonical_json_bytes_with_limit(&value, MAX_CHARTER_PENDING_SUBJECT_BYTES)?;
    assert_eq!(CharterPendingConsentV1::decode(&raw)?, good);
    for pointer in [
        "/terms/owner_term",
        "/terms/ownership",
        "/deadline",
        "/terms/offer_action",
    ] {
        let mut v = value.clone();
        *v.pointer_mut(pointer).ok_or("field")? = serde_json::Value::Null;
        assert!(serde_json::from_value::<CharterPendingConsentV1>(v).is_err());
    }
    let mut extra = value.clone();
    extra
        .get_mut("terms")
        .and_then(|v| v.as_object_mut())
        .ok_or("terms")?
        .insert("source".into(), json!("not authority"));
    assert!(serde_json::from_value::<CharterPendingConsentV1>(extra).is_err());
    let mut padded = raw.clone();
    padded.push(b' ');
    assert!(CharterPendingConsentV1::decode(&padded).is_err());
    assert!(
        CharterPendingConsentV1::decode(&vec![b' '; MAX_CHARTER_PENDING_SUBJECT_BYTES + 1])
            .is_err()
    );
    Ok(())
}
pub(super) fn maximum_consent() -> Result<CharterPendingConsentV1> {
    let mut row = sample()?;
    row.terms.charter = ObjId(u32::MAX);
    row.terms.seller = ObjId(u32::MAX - 1);
    row.terms.buyer = ObjId(u32::MAX - 2);
    row.terms.seller_account = AccountId(u64::MAX);
    row.payer = AccountId(u64::MAX - 1);
    row.terms.price = Amount::from_minor(i64::MAX);
    row.terms.title_revision = u64::MAX - 1;
    row.terms.ownership.generation = 65536;
    row.terms.owner_term.licence = u64::MAX;
    row.terms.owner_term.generation = u64::MAX;
    row.terms.owner_term.term_end = i64::MAX - 1;
    row.terms.owner_term.grace_end = i64::MAX;
    row.terms.offered_at_ic = ClockStamp {
        clock: ClockRef::WorldIc {
            generation: u64::MAX,
        },
        at: i64::MAX - 100,
    };
    row.accepted_at_ic = ClockStamp {
        clock: row.terms.offered_at_ic.clock.clone(),
        at: i64::MAX - 10,
    };
    row.terms.settle_seconds = 259200;
    row.terms.offered_at_utc = i64::MAX - 100;
    row.terms.expires_at_utc = i64::MAX;
    row.accepted_at_utc = i64::MAX - 10;
    row.deadline = row.terms.deadline(&row.accepted_at_ic)?;
    assert_eq!(row.deadline.at, i64::MAX - 1);
    row.terms.offer_action = DefinitionRef::new(
        row.terms.offer_action.key().clone(),
        2,
        DefinitionDigest::from_bytes([255; 32]),
    )?;
    row.effect = DefinitionRef::new(
        row.effect.key().clone(),
        1,
        DefinitionDigest::from_bytes([255; 32]),
    )?;
    // The instrument grammar has no artificial short-string ceiling. Exercise
    // the actual entire-subject bound with its longest accepted canonical ID.
    let mut low = 2;
    let mut high = MAX_CHARTER_PENDING_SUBJECT_BYTES + 1;
    while low + 1 < high {
        let middle = (low + high) / 2;
        let mut candidate = row.clone();
        candidate.terms.instrument = InstrumentId::new("i".repeat(middle)).ok_or("instrument")?;
        if candidate.validate().is_ok() {
            low = middle;
        } else {
            high = middle;
        }
    }
    row.terms.instrument = InstrumentId::new("i".repeat(low)).ok_or("instrument")?;
    row.validate()?;
    assert_eq!(
        serde_json::to_vec(&row)?.len(),
        MAX_CHARTER_PENDING_SUBJECT_BYTES
    );
    Ok(row)
}
#[test]
fn maximum_scalar_consent_and_saturating_term_cap_fit_exact_subject_limit() -> Result {
    let mut row = maximum_consent()?;
    let value = serde_json::to_value(&row)?;
    let raw =
        mush_core::canonical_json_bytes_with_limit(&value, MAX_CHARTER_PENDING_SUBJECT_BYTES)?;
    assert!(raw.len() <= MAX_CHARTER_PENDING_SUBJECT_BYTES);
    assert_eq!(CharterPendingConsentV1::decode(&raw)?, row);
    // Account zero is a real account identity, never a sentinel.
    row.payer = AccountId(0);
    row.validate()?;
    Ok(())
}

#[test]
fn retained_decision_keeps_deadline_clock_and_exact_term_refund_causes_separate() -> Result {
    let consent = sample()?;
    let good = CharterPendingDecisionV1 {
        format: 1,
        operation: token(5),
        action: action(SETTLE_ACTION, 1)?,
        actor: ObjId(1),
        at_ic: stamp(210),
        posted_at_utc: 120,
        reason: CharterPendingDecisionKindV1::DeadlineReached {
            next_transition: 9,
            open_owned: 2,
        },
    };
    good.matches_consent(&consent)?;
    let mut early = good.clone();
    early.at_ic.at = 209;
    assert!(early.matches_consent(&consent).is_err());
    let mut last = good.clone();
    last.reason = CharterPendingDecisionKindV1::LastClaimClosed { next_transition: 9 };
    last.at_ic.at = 110;
    last.matches_consent(&consent)?;
    let mut changed = good.clone();
    changed.action = action(REFUND_ACTION, 1)?;
    changed.reason = CharterPendingDecisionKindV1::ClockChanged;
    assert!(changed.matches_consent(&consent).is_err());
    changed.at_ic.clock = ClockRef::WorldIc { generation: 2 };
    changed.at_ic.at = 0;
    changed.matches_consent(&consent)?;
    let mut revoked = changed.clone();
    revoked.action = action("registry_term_revocation", 1)?;
    revoked.reason = CharterPendingDecisionKindV1::RegistryRevoked {
        sanction: revoked.operation,
        licence: consent.terms.owner_term.licence,
        generation: consent.terms.owner_term.generation,
        term_digest: consent.terms.owner_term.digest,
    };
    revoked.matches_consent(&consent)?;
    let mut crossed = revoked.clone();
    if let CharterPendingDecisionKindV1::RegistryRevoked { generation, .. } = &mut crossed.reason {
        *generation += 1;
    }
    assert!(crossed.matches_consent(&consent).is_err());
    crossed = revoked;
    crossed.posted_at_utc = consent.accepted_at_utc - 1;
    assert!(crossed.matches_consent(&consent).is_err());
    // None of these structurally valid values proves a claim census, actor
    // capability or sanction: actual registered owner preparation must join it.
    Ok(())
}
