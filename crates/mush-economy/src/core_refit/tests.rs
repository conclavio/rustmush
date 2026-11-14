#![cfg(test)]

//! The core refit's rig for the refit pair's Economy tests (doc 46 CT.7). The bodies are in
//! `crate::refit_twin::tests`, the bay refit's tests (doc 31 cut UD.4b) made generic over the
//! leg; this file says what the core refit's fixture is and what it alone refuses. The fixture
//! is CT.0's (`ct0_snapshot.rs`): a chartered `core_refit` under the M8b composed policy, with
//! a displaced core.
use super::ct0_snapshot::{composed, family, intent as fixture, reference};
use super::plans::FundingIntent;
use super::*;
use crate::refit_twin::Core;
use crate::refit_twin::tests::{Check, Rig};
use mush_core::{DefinitionDigest, ObjId};
use mush_testkit::fixtures::stamp;

impl Rig for Core {
    const OFFER_ACTION: &'static str = OFFER_ACTION;
    const RESERVATION_SECONDS: i64 = RESERVATION_SECONDS;
    const PAYER: crate::AccountId = crate::AccountId(9);
    const FEE: i64 = FITTING_MINOR;
    const PROVIDER: crate::AccountId = crate::AccountId(1);
    const EXCLUSIVE_WHILE_OPEN: bool = false;
    fn intent() -> FundingIntent {
        fixture(
            &composed().expect("the composed policy"),
            RefitKind::CoreRefit,
            family("line").expect("a family")[1].clone(),
            Some(ObjId(51)),
        )
        .expect("the fixture")
    }
    /// Install is the kernel's certified core exchange; Offer, Accept and Cancel are the
    /// core refit's own definitions, which the kernel's protocol pins by reference.
    fn kernel_agrees(p: &RefitPaymentPolicy) {
        assert_eq!(
            p.action_descriptor(INSTALL_ACTION).expect("Install"),
            mush_core::core_exchange::action_descriptor().expect("the kernel's exchange"),
        );
        for action in [OFFER_ACTION, ACCEPT_ACTION, CANCEL_ACTION] {
            p.action_descriptor(action).expect(action);
        }
        assert!(
            p.action_descriptor("refit_core").is_err(),
            "no action of another name"
        );
    }
    fn validate_policy(p: &RefitPaymentPolicy) -> Result<(), RefitError> {
        p.validate()
    }
    fn validate_subject(s: &RefitSubject) -> Result<(), RefitError> {
        s.validate()
    }
    fn bad_policies(p: &RefitPaymentPolicy) -> Vec<(&'static str, RefitPaymentPolicy)> {
        let ungovern = p.ungovern.clone().expect("the composed policy ungoverns");
        let mut wrong = p.clone();
        wrong.recipe = ungovern.recipe.clone();
        let mut same_site = p.clone();
        same_site.ungovern = Some(UngovernTerms {
            factory: p.factory,
            ..ungovern.clone()
        });
        let mut free = p.clone();
        free.ungovern = Some(UngovernTerms {
            fee_minor: 0,
            ..ungovern
        });
        let mut base = p.clone();
        base.coreless[0].family = 9;
        vec![
            ("the ungoverning recipe as the governed one", wrong),
            ("the ungoverning site is the governed one", same_site),
            ("an ungoverning fee of nothing", free),
            ("a coreless base of no family", base),
        ]
    }
    fn bad_subjects(s: &RefitSubject) -> Vec<(&'static str, Check, RefitSubject)> {
        vec![
            (
                "the incoming core is the hull",
                Check::Validate,
                RefitSubject {
                    incoming: s.hull,
                    ..s.clone()
                },
            ),
            (
                "the displaced core is the incoming one",
                Check::Validate,
                RefitSubject {
                    displaced: Some(s.incoming),
                    ..s.clone()
                },
            ),
            (
                "the site cannot pay itself",
                Check::Policy,
                RefitSubject {
                    payer: crate::AccountId(1),
                    ..s.clone()
                },
            ),
            (
                "a target the class's family does not have",
                Check::Policy,
                RefitSubject {
                    target: family("rig").expect("a family")[0].clone(),
                    ..s.clone()
                },
            ),
            (
                "the ungoverning recipe on a governed target",
                Check::Policy,
                RefitSubject {
                    kind: RefitKind::CoreUngovern,
                    ..s.clone()
                },
            ),
        ]
    }
    fn bad_wire(wire: &mut serde_json::Value) {
        wire["incoming"] = wire["hull"].clone();
    }
    fn spent(s: &RefitSubject) -> Vec<CommitToken> {
        vec![s.offer, s.core_acceptance]
    }
    fn rival(i: &FundingIntent) -> FundingIntent {
        let mut r = i.clone();
        r.subject.order = 2;
        r.subject.offer = CommitToken::from_bytes([5; 16]);
        r.subject.hull = ObjId(98);
        r.supplier.operation = r.subject.offer;
        r
    }
    fn elsewhere(i: &FundingIntent) -> FundingIntent {
        let mut r = i.clone();
        r.subject.incoming = ObjId(52);
        r
    }
    fn bad_suppliers(i: &FundingIntent) -> Vec<(&'static str, RefitOfferAttribution)> {
        let s = &i.supplier;
        vec![
            (
                "another offer",
                RefitOfferAttribution {
                    operation: CommitToken::from_bytes([6; 16]),
                    ..s.clone()
                },
            ),
            (
                "another offer's digest",
                RefitOfferAttribution {
                    digest: DefinitionDigest::from_bytes([6; 32]),
                    ..s.clone()
                },
            ),
            (
                "observed after the acceptance",
                RefitOfferAttribution {
                    observed_ic: stamp(1),
                    ..s.clone()
                },
            ),
            (
                "a window other than five minutes",
                RefitOfferAttribution {
                    expires_utc: s.observed_utc + 301,
                    ..s.clone()
                },
            ),
        ]
    }
    fn other_site() -> (FundingIntent, crate::AccountId, i64) {
        let i = fixture(
            &composed().expect("the composed policy"),
            RefitKind::CoreUngovern,
            family("line_twin").expect("a family")[1].clone(),
            None,
        )
        .expect("the ungoverning fixture");
        (i, crate::AccountId(2), 25_000)
    }
    fn config(i: &FundingIntent) -> CoreRefitConfig {
        CoreRefitConfig::new(
            i.policy.clone(),
            reference("industry", "owner_participant", "core_production").expect("an owner"),
            std::sync::Arc::new(Nothing),
        )
        .expect("a config")
    }
}

/// An Industry that has acknowledged nothing.
struct Nothing;
impl RefitOwnerAuthority for Nothing {
    fn acknowledged_offer(
        &self,
        _: &mush_core::World,
        _: &RefitSubject,
    ) -> Result<RefitOfferAttribution, RefitError> {
        Err(RefitError::Unavailable)
    }
    fn acknowledged_ready_order(
        &self,
        _: &mush_core::World,
        _: &RefitSubject,
    ) -> Result<RefitOrderAttribution, RefitError> {
        Err(RefitError::Unavailable)
    }
}

crate::refit_twin::tests::instantiate!(Core);

/// Every identity constant of the core leg against a literal. The shared rig and owner read
/// the constants, so a moved constant moves both sides of an ordinary test; this, and the CT.0
/// snapshot, is where the second copy of each string lives. The two records whose field names
/// are the leg's own are held to their JSON here as well.
#[test]
fn core_identity_strings_hold_todays_bytes() {
    use crate::refit_twin::Leg;
    use crate::refit_twin::plans::Phase;
    assert_eq!(<Core as Leg>::ACCEPT_ACTION, "accept_core_refit");
    assert_eq!(<Core as Leg>::INSTALL_ACTION, "install_core_component");
    assert_eq!(<Core as Leg>::CANCEL_ACTION, "cancel_core_refit");
    assert_eq!(<Core as Leg>::NAMESPACE, "industry.core_refit.v1");
    assert_eq!(<Core as Leg>::HOLD_PURPOSE, "industry.core_refit");
    assert_eq!(<Core as Leg>::MAX_ORDERS, 8192);
    assert_eq!(<Core as Leg>::HISTORY_BYTES, 16_384);
    assert_eq!(
        <Core as Leg>::REFUSED,
        "core refit money or accepted authority disagrees"
    );
    assert_eq!(<Core as Leg>::PLAN_DESCRIPTION, "Certified core fitting");
    assert_eq!(<Core as Leg>::LINE_MEMO, "core fitting");
    assert_eq!(
        <Core as Leg>::HISTORIES_EXPECTING,
        "bounded distinct core refit histories"
    );
    assert_eq!(<Core as Leg>::HISTORY_LIMIT, "core history limit");
    assert_eq!(
        <Core as Leg>::HISTORY_DUPLICATE,
        "core history duplicate or limit"
    );
    assert_eq!(<Core as Leg>::HISTORY_KEY, "core history key");
    assert_eq!(
        <Core as Leg>::INVALID.to_string(),
        "core refit evidence is malformed"
    );
    assert_eq!(
        <Core as Leg>::INTEGRITY.to_string(),
        "core refit evidence disagrees"
    );
    assert_eq!(
        <Core as Leg>::INCOMPLETE.to_string(),
        "core refit history is incomplete"
    );
    assert_eq!(
        <Core as Leg>::CAPACITY.to_string(),
        "core refit capacity is exhausted"
    );
    assert_eq!(crate::refit_twin::plans::CORRELATION_BYTES, 16_384);
    let d = DefinitionDigest::from_bytes([1; 32]);
    assert_eq!(
        serde_json::to_value(<Core as Leg>::installed(d)).expect("encodes"),
        serde_json::json!({"outcome": "installed", "replacement": d})
    );
    assert_eq!(
        serde_json::to_value(<Core as Leg>::cancelled()).expect("encodes"),
        serde_json::json!({"outcome": "cancelled"})
    );
    let at = stamp(7200);
    let proof = <Core as Leg>::seal(Phase::Install {
        format: 1,
        acceptance: CommitToken::from_bytes([1; 16]),
        commit: CommitToken::from_bytes([2; 16]),
        action: <Core as Leg>::action_ref(
            &<Core as Rig>::intent().policy,
            <Core as Leg>::INSTALL_ACTION,
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
            "replacement"
        ]
    );
    assert_eq!(wire["phase"], "install");
    assert_eq!(wire["replacement"], serde_json::json!(d));
}
