#![cfg(test)]

//! Doc 31 cut UD.4b: the bay fee's terms and its fixed plan alphabet. The
//! live hold, capture and release run through the Industry bay provider's
//! integration rig, which composes this leg with the kernel protocol.
use super::plans::{self, FundingIntent};
use super::*;
use crate::core_integration::ScopedCoreAccount;
use crate::plan::HoldOp;
use mush_core::{
    ClockRef, ClockStamp, DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind,
    DefinitionRef, DefinitionRegistryBuilder, ObjId, OwnerId,
};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};

fn reference(owner: &str, kind: &str, id: &str) -> DefinitionRef {
    DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new(owner).expect("an owner"),
            DefinitionDescriptor::new(
                DefinitionKind::new(kind).expect("a kind"),
                DefinitionId::new(id).expect("an id"),
                1,
                serde_json::json!({"fixture": id}),
            )
            .expect("a descriptor"),
        )
        .expect("registers")
}
fn site(facility: u32, n: u64) -> BaySite {
    BaySite {
        facility: ObjId(facility),
        provider: ScopedCoreAccount {
            account: crate::AccountId(n),
            organization: OrgId(n),
            scope: OrgScopeId(n),
        },
    }
}
fn policy() -> BayPaymentPolicy {
    BayPaymentPolicy {
        issuer: SystemPrincipalId::new("hardware.factory").expect("an issuer"),
        instrument: crate::InstrumentId::new("tacsim.credit").expect("an instrument"),
        recipe: reference("industry", "recipe", "bay_refit"),
        bays: vec![site(40, 1), site(41, 1)],
        max_fee_minor: 50_000,
    }
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}
fn subject(p: &BayPaymentPolicy) -> BaySubject {
    BaySubject {
        order: 1,
        intent: DefinitionDigest::from_bytes([1; 32]),
        offer: CommitToken::from_bytes([2; 16]),
        offer_digest: DefinitionDigest::from_bytes([3; 32]),
        requester: ObjId(7),
        payer: crate::AccountId(9),
        hull: ObjId(99),
        unit_revision: 3,
        before: reference("tacsim", "unit", "line_tank"),
        target: reference("tacsim", "unit", "line_tank_laser"),
        facility: ObjId(40),
        fee_minor: 12_500,
        policy: p.digest().expect("a digest"),
        accepted_at: stamp(0),
        expires_at: stamp(RESERVATION_SECONDS),
    }
}
fn intent() -> FundingIntent {
    let p = policy();
    let s = subject(&p);
    FundingIntent {
        supplier: BayOfferAttribution {
            operation: s.offer,
            digest: s.offer_digest,
            action: p.action_ref(OFFER_ACTION).expect("the offer"),
            operator: ObjId(8),
            observed_ic: stamp(0),
            observed_utc: 100,
            expires_utc: 400,
        },
        subject: s,
        policy: p,
    }
}

/// The paid actions are the kernel's own: Economy reads the one definition
/// Industry and the kernel read, under the Industry policy of its bays.
#[test]
fn ud4b_the_fee_policy_names_the_kernels_actions() {
    let p = policy();
    let industry = p.industry_policy();
    for (i, action) in mush_core::bay_refit_protocol::ACTIONS.iter().enumerate() {
        let ours = p.action_descriptor(action).expect("a descriptor");
        let kernel =
            mush_core::bay_refit_protocol::action_descriptor(&industry, i).expect("the kernel's");
        assert_eq!(ours, kernel, "{action}");
    }
    p.action_ref(OFFER_ACTION).expect("Industry's offer");
    let mut unsorted = policy();
    unsorted.bays.reverse();
    assert!(unsorted.validate().is_err(), "bays ascend by facility");
    let mut free = policy();
    free.max_fee_minor = 0;
    assert!(free.validate().is_err());
    let mut wrong = policy();
    wrong.recipe = reference("industry", "recipe", "core_refit");
    assert!(wrong.validate().is_err());
}

/// The subject's fee is a quote under the policy's ceiling, at one of its
/// bays, paid by someone other than the bay.
#[test]
fn ud4b_the_quote_is_bounded_and_paid_to_its_bay() {
    let p = policy();
    let s = subject(&p);
    s.matches_policy(&p).expect("matches");
    let over = BaySubject {
        fee_minor: 50_001,
        ..s.clone()
    };
    assert!(over.matches_policy(&p).is_err());
    let elsewhere = BaySubject {
        facility: ObjId(42),
        ..s.clone()
    };
    assert!(elsewhere.matches_policy(&p).is_err());
    let own = BaySubject {
        payer: crate::AccountId(1),
        ..s.clone()
    };
    assert!(own.matches_policy(&p).is_err(), "the bay cannot pay itself");
    let same = BaySubject {
        target: s.before.clone(),
        ..s.clone()
    };
    assert!(same.validate().is_err(), "a refit changes the root");
    let encoded = serde_json::to_value(&s).expect("encodes");
    assert_eq!(
        serde_json::from_value::<BaySubject>(encoded).expect("decodes"),
        s
    );
    let mut bad = serde_json::to_value(&s).expect("encodes");
    bad["fee_minor"] = serde_json::json!(0);
    assert!(
        serde_json::from_value::<BaySubject>(bad).is_err(),
        "decoding validates"
    );
}

/// Accept holds the quoted fee for the order; Install captures exactly it to
/// the bay's provider; Cancel releases it; each under its own key.
#[test]
fn ud4b_the_plans_hold_capture_and_release_the_quote() {
    let intent = intent();
    let p = &intent.policy;
    let accept = CommitToken::from_bytes([10; 16]);
    let funding = plans::funding(
        &intent,
        accept,
        &p.action_ref(ACCEPT_ACTION).expect("accept"),
    )
    .expect("a funding plan");
    assert_eq!(funding.idempotency.namespace, NAMESPACE);
    assert!(funding.requested_lines.is_empty());
    let [
        HoldOp::Place {
            account,
            amount,
            purpose,
            ..
        },
    ] = funding.hold_ops.as_slice()
    else {
        panic!("one hold placed");
    };
    assert_eq!((*account, amount.minor()), (crate::AccountId(9), 12_500));
    assert_eq!(
        purpose,
        &crate::HoldPurpose::Reservation {
            kind: HOLD_PURPOSE.into(),
            id: 1
        }
    );
    assert!(
        plans::funding(
            &intent,
            accept,
            &p.action_ref(CANCEL_ACTION).expect("cancel")
        )
        .is_err(),
        "funding is Accept's alone"
    );
    let hold = crate::HoldId(5);
    let install = CommitToken::from_bytes([11; 16]);
    let refit = DefinitionDigest::from_bytes([12; 32]);
    let captured = plans::terminal(
        &intent,
        accept,
        hold,
        install,
        &p.action_ref(INSTALL_ACTION).expect("install"),
        &stamp(7200),
        Some(refit),
    )
    .expect("an install plan");
    let lines: Vec<_> = captured
        .requested_lines
        .iter()
        .map(|l| (l.account, l.delta.minor()))
        .collect();
    assert_eq!(
        lines,
        [
            (crate::AccountId(9), -12_500),
            (crate::AccountId(1), 12_500)
        ]
    );
    assert!(matches!(
        captured.hold_ops.as_slice(),
        [HoldOp::Capture { hold: h, amount }] if *h == hold && amount.minor() == 12_500
    ));
    let released = plans::terminal(
        &intent,
        accept,
        hold,
        install,
        &p.action_ref(CANCEL_ACTION).expect("cancel"),
        &stamp(7200),
        None,
    )
    .expect("a cancel plan");
    assert!(released.requested_lines.is_empty());
    assert!(matches!(released.hold_ops.as_slice(), [HoldOp::Release { hold: h }] if *h == hold));
    assert_ne!(funding.idempotency, captured.idempotency);
    assert_ne!(captured.idempotency, released.idempotency);
    assert!(
        plans::terminal(
            &intent,
            accept,
            hold,
            install,
            &p.action_ref(INSTALL_ACTION).expect("install"),
            &stamp(RESERVATION_SECONDS),
            Some(refit),
        )
        .is_err(),
        "no install after the reservation lapses"
    );
}
