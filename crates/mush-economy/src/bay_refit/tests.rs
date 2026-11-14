#![cfg(test)]

//! Doc 31 cut UD.4b: the bay fee's terms and its fixed plan alphabet. The
//! bodies are in `crate::refit_twin::tests`, run for the bay here and for the core
//! refit in `core_refit::tests` (doc 46 CT.7); this file is the bay's rig. The
//! live hold, capture and release run through the Industry bay provider's
//! integration rig, which composes this leg with the kernel protocol.
use super::plans::FundingIntent;
use super::*;
use crate::core_integration::ScopedCoreAccount;
use crate::refit_twin::Bay;
use crate::refit_twin::tests::{Check, Rig};
use mush_core::{
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, ObjId, OwnerId,
};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use mush_testkit::fixtures::stamp;

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

impl Rig for Bay {
    const OFFER_ACTION: &'static str = OFFER_ACTION;
    const RESERVATION_SECONDS: i64 = RESERVATION_SECONDS;
    const PAYER: crate::AccountId = crate::AccountId(9);
    const FEE: i64 = 12_500;
    const PROVIDER: crate::AccountId = crate::AccountId(1);
    const EXCLUSIVE_WHILE_OPEN: bool = true;
    fn intent() -> FundingIntent {
        intent()
    }
    fn kernel_agrees(p: &BayPaymentPolicy) {
        let industry = p.industry_policy();
        for (i, action) in mush_core::bay_refit_protocol::ACTIONS.iter().enumerate() {
            let ours = p.action_descriptor(action).expect("a descriptor");
            let kernel = mush_core::bay_refit_protocol::action_descriptor(&industry, i)
                .expect("the kernel's");
            assert_eq!(ours, kernel, "{action}");
        }
        p.action_ref(OFFER_ACTION).expect("Industry's offer");
    }
    fn validate_policy(p: &BayPaymentPolicy) -> Result<(), BayRefitError> {
        p.validate()
    }
    fn validate_subject(s: &BaySubject) -> Result<(), BayRefitError> {
        s.validate()
    }
    fn bad_policies(p: &BayPaymentPolicy) -> Vec<(&'static str, BayPaymentPolicy)> {
        let mut unsorted = p.clone();
        unsorted.bays.reverse();
        let mut free = p.clone();
        free.max_fee_minor = 0;
        let mut wrong = p.clone();
        wrong.recipe = reference("industry", "recipe", "core_refit");
        vec![
            ("bays ascend by facility", unsorted),
            ("a fee ceiling of nothing", free),
            ("the core refit's recipe", wrong),
        ]
    }
    fn bad_subjects(s: &BaySubject) -> Vec<(&'static str, Check, BaySubject)> {
        vec![
            (
                "over the ceiling",
                Check::Policy,
                BaySubject {
                    fee_minor: 50_001,
                    ..s.clone()
                },
            ),
            (
                "a bay the policy does not have",
                Check::Policy,
                BaySubject {
                    facility: ObjId(42),
                    ..s.clone()
                },
            ),
            (
                "the bay cannot pay itself",
                Check::Policy,
                BaySubject {
                    payer: crate::AccountId(1),
                    ..s.clone()
                },
            ),
            (
                "a refit changes the root",
                Check::Validate,
                BaySubject {
                    target: s.before.clone(),
                    ..s.clone()
                },
            ),
        ]
    }
    fn bad_wire(wire: &mut serde_json::Value) {
        wire["fee_minor"] = serde_json::json!(0);
    }
    fn spent(s: &BaySubject) -> Vec<CommitToken> {
        vec![s.offer]
    }
    fn rival(i: &FundingIntent) -> FundingIntent {
        let mut r = i.clone();
        r.subject.order = 2;
        r.subject.offer = CommitToken::from_bytes([5; 16]);
        r.supplier.operation = r.subject.offer;
        r
    }
    fn elsewhere(i: &FundingIntent) -> FundingIntent {
        let mut r = i.clone();
        r.subject.hull = ObjId(98);
        r
    }
    fn bad_suppliers(i: &FundingIntent) -> Vec<(&'static str, BayOfferAttribution)> {
        let s = &i.supplier;
        vec![
            (
                "another offer",
                BayOfferAttribution {
                    operation: CommitToken::from_bytes([6; 16]),
                    ..s.clone()
                },
            ),
            (
                "another offer's digest",
                BayOfferAttribution {
                    digest: DefinitionDigest::from_bytes([6; 32]),
                    ..s.clone()
                },
            ),
            (
                "observed after the acceptance",
                BayOfferAttribution {
                    observed_ic: stamp(1),
                    ..s.clone()
                },
            ),
            (
                "a window other than five minutes",
                BayOfferAttribution {
                    expires_utc: s.observed_utc + 301,
                    ..s.clone()
                },
            ),
        ]
    }
    fn config(i: &FundingIntent) -> BayRefitConfig {
        BayRefitConfig::new(
            i.policy.clone(),
            reference("industry", "owner_participant", "bay_refit"),
            std::sync::Arc::new(Nothing),
        )
        .expect("a config")
    }
}

/// An Industry that has acknowledged nothing.
struct Nothing;
impl BayOwnerAuthority for Nothing {
    fn acknowledged_offer(
        &self,
        _: &mush_core::World,
        _: &BaySubject,
    ) -> Result<BayOfferAttribution, BayRefitError> {
        Err(BayRefitError::Unavailable)
    }
    fn acknowledged_ready_order(
        &self,
        _: &mush_core::World,
        _: &BaySubject,
    ) -> Result<BayOrderAttribution, BayRefitError> {
        Err(BayRefitError::Unavailable)
    }
}

crate::refit_twin::tests::instantiate!(Bay);

/// Every identity constant of the bay leg against a literal. The shared rig and owner read
/// the constants, so a moved constant moves both sides of an ordinary test; this, and the CT.0
/// snapshot, is where the second copy of each string lives. The two records whose field names
/// are the leg's own are held to their JSON here as well.
#[test]
fn bay_identity_strings_hold_todays_bytes() {
    use crate::refit_twin::Leg;
    use crate::refit_twin::plans::Phase;
    assert_eq!(<Bay as Leg>::ACCEPT_ACTION, "accept_bay_refit");
    assert_eq!(<Bay as Leg>::INSTALL_ACTION, "install_bay_refit");
    assert_eq!(<Bay as Leg>::CANCEL_ACTION, "cancel_bay_refit");
    assert_eq!(<Bay as Leg>::NAMESPACE, "industry.bay_refit.v1");
    assert_eq!(<Bay as Leg>::HOLD_PURPOSE, "industry.bay_refit");
    assert_eq!(<Bay as Leg>::MAX_ORDERS, 8192);
    assert_eq!(<Bay as Leg>::HISTORY_BYTES, 16_384);
    assert_eq!(
        <Bay as Leg>::REFUSED,
        "bay refit money or accepted authority disagrees"
    );
    assert_eq!(<Bay as Leg>::PLAN_DESCRIPTION, "Bay refit");
    assert_eq!(<Bay as Leg>::LINE_MEMO, "bay refit");
    assert_eq!(
        <Bay as Leg>::HISTORIES_EXPECTING,
        "bounded distinct bay refit histories"
    );
    assert_eq!(<Bay as Leg>::HISTORY_LIMIT, "bay history limit");
    assert_eq!(
        <Bay as Leg>::HISTORY_DUPLICATE,
        "bay history duplicate or limit"
    );
    assert_eq!(<Bay as Leg>::HISTORY_KEY, "bay history key");
    assert_eq!(
        <Bay as Leg>::INVALID.to_string(),
        "bay refit evidence is malformed"
    );
    assert_eq!(
        <Bay as Leg>::INTEGRITY.to_string(),
        "bay refit evidence disagrees"
    );
    assert_eq!(
        <Bay as Leg>::INCOMPLETE.to_string(),
        "bay refit history is incomplete"
    );
    assert_eq!(
        <Bay as Leg>::CAPACITY.to_string(),
        "bay refit capacity is exhausted"
    );
    assert_eq!(crate::refit_twin::plans::CORRELATION_BYTES, 16_384);
    let d = DefinitionDigest::from_bytes([1; 32]);
    assert_eq!(
        serde_json::to_value(<Bay as Leg>::installed(d)).expect("encodes"),
        serde_json::json!({"outcome": "installed", "refit": d})
    );
    assert_eq!(
        serde_json::to_value(<Bay as Leg>::cancelled()).expect("encodes"),
        serde_json::json!({"outcome": "cancelled"})
    );
    let at = stamp(7200);
    let proof = <Bay as Leg>::seal(Phase::Install {
        format: 1,
        acceptance: CommitToken::from_bytes([1; 16]),
        commit: CommitToken::from_bytes([2; 16]),
        action: <Bay as Leg>::action_ref(
            &<Bay as Rig>::intent().policy,
            <Bay as Leg>::INSTALL_ACTION,
        )
        .expect("install"),
        at: at.clone(),
        part: d,
    });
    let wire = serde_json::to_value(&proof).expect("encodes");
    let keys: Vec<_> = wire
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        keys,
        [
            "acceptance",
            "action",
            "at",
            "commit",
            "format",
            "phase",
            "refit"
        ]
    );
    assert_eq!(wire["phase"], "install");
    assert_eq!(wire["refit"], serde_json::json!(d));
}

mod ct0_snapshot;
