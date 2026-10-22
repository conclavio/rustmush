//! Structural/size checks only. These values are never owner authority.
use super::*;
use crate::managed_entitlement::{
    GrossRecipientCut, Recipient, RoyaltyRecipientCut, RoyaltyTitleRecipient,
};
use crate::{AccountId, Amount, InstrumentId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, ObjId};
use mush_org::PrincipalId;
use serde_json::json;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn digest() -> DefinitionDigest {
    DefinitionDigest::from_bytes([255; 32])
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}
fn action(id: &str) -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("market")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("action")?,
            DefinitionId::new(id)?,
            1,
            json!({"fixture":"inactive_paid_resale"}),
        )?,
    )?)
}
fn recipient(account: u64, owner: u32) -> Recipient {
    Recipient {
        account: AccountId(account),
        owner: PrincipalId::Object(ObjId(owner)),
    }
}
pub(super) fn sample() -> Result<PaidResaleConsentV1> {
    let terms = PaidResaleTermsV1 {
        format: 1,
        offer: token(3),
        offer_action: action(OFFER)?,
        basis: PaidResaleBasisV1 {
            licence: 1,
            charter: ObjId(8),
            line: 1,
            birth: PaidBirthRefV1 {
                route: PaidBirthRouteV1::Entitlement,
                operation: token(1),
                digest: digest(),
            },
            holder: PaidHolderRefV1 {
                generation: 1,
                operation: token(1),
                digest: digest(),
            },
            term: PaidResaleTermBasisV1 {
                generation: 1,
                operation: token(1),
                digest: digest(),
                clock: stamp(0).clock,
                window: PaidResaleWindowV1 {
                    format: 1,
                    eligibility_start: 0,
                    issued_at: 0,
                    term_end: 1000,
                    grace_end: 1100,
                },
                terms_digest: digest(),
                policy_digest: digest(),
            },
            economics_digest: digest(),
            royalty_chain_digest: digest(),
        },
        seller: ObjId(1),
        buyer: ObjId(2),
        instrument: InstrumentId::new("cr").ok_or("instrument")?,
        price: Amount::from_minor(103),
        registry: GrossRecipientCut {
            permille: 50,
            recipient: Some(recipient(1, 9)),
        },
        venue: Some(GrossRecipientCut {
            permille: 50,
            recipient: Some(recipient(2, 10)),
        }),
        parents: vec![
            RoyaltyRecipientCut {
                charter: ObjId(31),
                permille: 100,
                title: Some(RoyaltyTitleRecipient {
                    title_revision: 1,
                    recipient: recipient(3, 11),
                }),
            },
            RoyaltyRecipientCut {
                charter: ObjId(32),
                permille: 100,
                title: Some(RoyaltyTitleRecipient {
                    title_revision: 1,
                    recipient: recipient(4, 12),
                }),
            },
            RoyaltyRecipientCut {
                charter: ObjId(33),
                permille: 100,
                title: Some(RoyaltyTitleRecipient {
                    title_revision: 1,
                    recipient: recipient(5, 2),
                }),
            },
        ],
        remainder: recipient(0, 1),
        offered_at_ic: stamp(100),
        offered_at_utc: 100,
        expires_at_utc: 200,
        settle_seconds: SETTLEMENT_IC_SECONDS,
    };
    let c = PaidResaleConsentV1 {
        format: 1,
        operation: token(4),
        effect: action(FUND)?,
        offer_digest: digest(),
        payer: AccountId(5),
        mode: PaidResaleModeV1::Pending,
        accepted_at_ic: stamp(110),
        accepted_at_utc: 110,
        deadline: terms.deadline(&stamp(110))?,
        claims: PaidResaleClaimsV1 {
            next_transition: 5,
            open_paid: 1,
        },
        terms,
    };
    c.validate()?;
    Ok(c)
}
#[test]
fn resale_gross_split_preserves_six_roles_payer_credit_and_floor_before_merge() -> Result {
    let c = sample()?;
    let payouts = c.terms.payouts(c.payer)?;
    check_eq!(payouts.len(), 6);
    let amount = |id| {
        payouts
            .iter()
            .find(|p| p.account == AccountId(id))
            .map(|p| p.amount.minor())
    };
    check_eq!(amount(0), Some(63));
    check_eq!(amount(5), Some(10));
    check_eq!(payouts.iter().map(|p| p.amount.minor()).sum::<i64>(), 103);
    let mut merged = c.clone();
    merged.terms.registry.recipient = Some(recipient(3, 11));
    merged.terms.venue = Some(GrossRecipientCut {
        permille: 50,
        recipient: Some(recipient(3, 11)),
    });
    let p = merged.terms.payouts(merged.payer)?;
    check_eq!(p.len(), 4);
    check_eq!(
        p.iter()
            .find(|p| p.account == AccountId(3))
            .map(|p| p.amount.minor()),
        Some(20)
    );
    merged.terms.price = Amount::from_minor(1);
    let p = merged.terms.payouts(merged.payer)?;
    check_eq!(p.len(), 1);
    check_eq!(p[0].amount.minor(), 1);
    // A floor of zero does not excuse crossed account ownership.
    merged.terms.registry.recipient = Some(recipient(3, 99));
    check!(merged.validate().is_err());
    Ok(())
}
#[test]
fn resale_immediate_and_pending_are_disjoint_and_preserve_lapsed_dates() -> Result {
    let c = sample()?;
    let mut immediate = c.clone();
    immediate.effect = action(ACCEPT)?;
    immediate.mode = PaidResaleModeV1::Immediate;
    check!(immediate.validate().is_err());
    immediate.claims.open_paid = 0;
    immediate.validate()?;
    let before = immediate.terms.basis.term.clone();
    immediate.terms.offered_at_ic = stamp(1200);
    immediate.accepted_at_ic = stamp(1210);
    immediate.deadline = immediate.terms.deadline(&immediate.accepted_at_ic)?;
    immediate.validate()?;
    check_eq!(immediate.deadline.at, 1000);
    check!(immediate.deadline_reached(&immediate.accepted_at_ic)?);
    check_eq!(immediate.terms.basis.term, before);
    immediate.mode = PaidResaleModeV1::Pending;
    immediate.effect = action(FUND)?;
    check!(immediate.validate().is_err());
    Ok(())
}
#[test]
fn resale_strict_nested_nulls_parent_bound_and_unknown_fields_refuse() -> Result {
    let c = sample()?;
    let good = serde_json::to_value(&c)?;
    for path in [
        vec!["terms", "venue"],
        vec!["terms", "registry", "recipient"],
        vec!["terms", "parents", "0", "title"],
    ] {
        let mut v = good.clone();
        if path[1] == "parents" {
            v["terms"]["parents"][0]
                .as_object_mut()
                .ok_or("object")?
                .remove("title");
        } else if path.len() == 2 {
            v["terms"].as_object_mut().ok_or("object")?.remove(path[1]);
        } else {
            v["terms"][path[1]]
                .as_object_mut()
                .ok_or("object")?
                .remove(path[2]);
        }
        check!(serde_json::from_value::<PaidResaleConsentV1>(v).is_err());
    }
    let mut v = good.clone();
    let extra = v["terms"]["parents"][0].clone();
    v["terms"]["parents"]
        .as_array_mut()
        .ok_or("array")?
        .push(extra);
    check!(serde_json::from_value::<PaidResaleConsentV1>(v).is_err());
    let mut v = good;
    v["terms"]["basis"]["holder"]["authority"] = json!(true);
    check!(serde_json::from_value::<PaidResaleConsentV1>(v).is_err());
    let raw = subject::bounded(&c, MAX_SUBJECT_BYTES)?;
    check_eq!(PaidResaleConsentV1::decode(&raw)?, c);
    let mut noncanonical = vec![b' '];
    noncanonical.extend(raw);
    check!(PaidResaleConsentV1::decode(&noncanonical).is_err());
    Ok(())
}
#[test]
fn resale_rejects_cut_overflow_cycles_and_final_holder_generation() -> Result {
    let c = sample()?;
    for n in 0..8 {
        let mut v = c.clone();
        match n {
            0 => v.terms.parents[0].permille = 101,
            1 => v.terms.registry.permille = 701,
            2 => v.terms.parents[0].charter = v.terms.basis.charter,
            3 => v.terms.basis.holder.generation = MAX_HOLDER_GENERATION,
            4 => v.terms.price = Amount::ZERO,
            5 => v.payer = v.terms.remainder.account,
            6 => v.terms.basis.holder.operation = token(9),
            _ => v.terms.settle_seconds = 1,
        }
        check!(v.validate().is_err());
    }
    let mut max = c;
    max.terms.price = Amount::from_minor(i64::MAX);
    max.validate()?;
    check_eq!(
        max.terms
            .payouts(max.payer)?
            .iter()
            .map(|p| i128::from(p.amount.minor()))
            .sum::<i128>(),
        i128::from(i64::MAX)
    );
    Ok(())
}
fn decision(
    c: &PaidResaleConsentV1,
    reason: PaidResaleDecisionKindV1,
) -> Result<PaidResaleDecisionV1> {
    Ok(PaidResaleDecisionV1 {
        format: 1,
        operation: token(5),
        action: action(if reason.settles() { SETTLE } else { REFUND })?,
        actor: ObjId(1),
        at_ic: stamp(c.deadline.at),
        posted_at_utc: 120,
        reason,
    })
}
#[test]
fn resale_deadline_last_close_and_refund_causes_keep_exact_term_bounds() -> Result {
    let c = sample()?;
    let mut d = decision(
        &c,
        PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 6 },
    )?;
    d.at_ic = stamp(111);
    d.matches_consent(&c)?;
    d.reason = PaidResaleDecisionKindV1::LastClaimClosed { next_transition: 5 };
    check!(d.matches_consent(&c).is_err());
    let mut d = decision(
        &c,
        PaidResaleDecisionKindV1::DeadlineReached {
            next_transition: 5,
            open_paid: 1,
        },
    )?;
    d.matches_consent(&c)?;
    d.at_ic.at -= 1;
    check!(d.matches_consent(&c).is_err());
    let mut d = decision(
        &c,
        PaidResaleDecisionKindV1::RegistryRevoked {
            sanction: token(8),
            licence: 1,
            generation: 1,
            term_digest: c.terms.basis.term.digest,
        },
    )?;
    d.at_ic = stamp(1099);
    d.matches_consent(&c)?;
    d.at_ic = stamp(c.accepted_at_ic.at - 1);
    check!(d.matches_consent(&c).is_err());
    d.at_ic = stamp(c.accepted_at_ic.at);
    d.matches_consent(&c)?;
    d.at_ic = stamp(1100);
    check!(
        d.matches_consent(&c).is_err(),
        "expired sanction cannot ban property"
    );
    let mut d = decision(&c, PaidResaleDecisionKindV1::ClockChanged {})?;
    check!(d.matches_consent(&c).is_err());
    d.at_ic.clock = ClockRef::WorldIc { generation: 2 };
    d.matches_consent(&c)?;
    d.reason = PaidResaleDecisionKindV1::RecipientUnavailable {};
    check!(d.matches_consent(&c).is_err());
    Ok(())
}
#[test]
fn resale_widest_scalar_subject_and_grace_issue_stay_bounded() -> Result {
    let mut c = sample()?;
    c.terms.basis.licence = u64::MAX;
    c.terms.basis.line = u64::MAX;
    c.terms.basis.term.generation = u64::MAX;
    c.terms.basis.holder.generation = 65536;
    c.terms.basis.holder.operation = token(9);
    c.terms.price = Amount::from_minor(i64::MAX);
    c.terms.basis.term.window.issued_at = 1050; // Lawful historical Grace issue.
    c.claims.next_transition = u64::MAX;
    c.claims.open_paid = 32768;
    c.validate()?;
    let raw = subject::bounded(&c, MAX_SUBJECT_BYTES)?;
    check_eq!(PaidResaleConsentV1::decode(&raw)?, c);
    check!(raw.len() < MAX_SUBJECT_BYTES);
    Ok(())
}
