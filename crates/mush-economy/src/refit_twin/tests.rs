#![cfg(test)]

//! The refit pair's Economy tests, one body run for both legs (doc 46 CT.7).
//!
//! These were the bay refit's (doc 31 cut UD.4b): the fee policy names the kernel's actions,
//! the subject is bounded and pays its site, and the plans hold, capture and release the fee.
//! Each is a generic function over [`Rig`] here; [`instantiate!`] names it as a `#[test]` in a
//! leg's own module, so `bay_refit::tests::<name>` is the path it always was and
//! `core_refit::tests::<name>` is its new twin. What differs between the legs (which policies
//! and subjects are bad, which kernel definitions the actions must equal) is the rig's.
//!
//! The live hold, capture and release run through Industry's integration rigs, which compose
//! the leg with the kernel protocol; these tests stop at the closed money alphabet, the
//! records and the replayed history.
use super::plans::{self, FundingIntent};
use super::receipt::{FundingReceipt, PaymentHistory, ReceiptAudit, TerminalReceipt};
use super::{Leg, Subject};
use crate::plan::{EconomyPlan, HoldOp};
use crate::record::{Hold, HoldPurpose, HoldStatus};
use crate::{AccountId, HoldId, TransactionId};
use mush_core::{CommitToken, DefinitionDigest};
use mush_org::PrincipalId;
use mush_testkit::fixtures::stamp;

/// Which check a bad subject must fail.
pub(crate) enum Check {
    /// The subject's own `validate`.
    Validate,
    /// `matches_policy` under the fixture's policy.
    Policy,
}

/// **A leg under test**: its fixture and what it alone refuses.
pub(crate) trait Rig: Leg {
    const OFFER_ACTION: &'static str;
    const RESERVATION_SECONDS: i64;
    /// What the fixture's Install must move: the payer's account, the fee and the provider's
    /// account.
    const PAYER: AccountId;
    const FEE: i64;
    const PROVIDER: AccountId;
    /// An accepted order's intent under the leg's policy: order 1, accepted at IC 0.
    fn intent() -> FundingIntent<Self>;
    /// Every paid action's descriptor is the one the kernel reads.
    fn kernel_agrees(policy: &Self::Policy);
    fn validate_policy(policy: &Self::Policy) -> Result<(), Self::Error>;
    fn validate_subject(subject: &Self::Subject) -> Result<(), Self::Error>;
    /// Policies the family refuses, each named.
    fn bad_policies(policy: &Self::Policy) -> Vec<(&'static str, Self::Policy)>;
    /// Subjects the family refuses, each named, with the check that refuses it.
    fn bad_subjects(subject: &Self::Subject) -> Vec<(&'static str, Check, Self::Subject)>;
    /// Make the subject's wire one its decode refuses.
    fn bad_wire(wire: &mut serde_json::Value);
    /// The commit tokens the subject already spent: its offer, and for the core refit its
    /// manufacture acceptance.
    fn spent(subject: &Self::Subject) -> Vec<CommitToken>;
    /// Another order that shares the fixture's exclusive object ([`Leg::exclusive`]): the bay's
    /// hull, the core refit's incoming core. Its order, offer and payer hold differ.
    fn rival(intent: &FundingIntent<Self>) -> FundingIntent<Self>;
    /// The same order with its exclusive object changed.
    fn elsewhere(intent: &FundingIntent<Self>) -> FundingIntent<Self>;
    /// Whether the exclusive object is held while the order is open (the bay) or once it is
    /// installed (the core refit).
    const EXCLUSIVE_WHILE_OPEN: bool;
    /// Suppliers' attributions the family refuses for the fixture's subject, each named.
    fn bad_suppliers(intent: &FundingIntent<Self>) -> Vec<(&'static str, Self::Supplier)>;
    /// The leg composed with no definitions yet (`cleanup_ready` must refuse it).
    fn config(intent: &FundingIntent<Self>) -> Self::Config;
}

/// Name each body as a `#[test]` of the leg `$leg`, in the module that expands it.
macro_rules! instantiate {
    ($leg:ty) => {
        crate::refit_twin::tests::instantiate!(
            $leg;
            ud4b_the_fee_policy_names_the_kernels_actions,
            ud4b_the_quote_is_bounded_and_paid_to_its_bay,
            ud4b_the_plans_hold_capture_and_release_the_quote,
            ct7_a_spent_commit_posts_no_plan,
            ct7_a_replayed_history_keeps_the_legs_exclusive_object,
            ct7_the_supplier_attributes_the_offer_taken,
            ct7_an_uncomposed_leg_is_not_ready,
        );
    };
    ($leg:ty; $($name:ident),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                crate::refit_twin::tests::$name::<$leg>();
            }
        )+
    };
}
pub(crate) use instantiate;

/// The paid actions are the kernel's own: Economy reads the one definition Industry and the
/// kernel read, under the leg's policy.
pub(crate) fn ud4b_the_fee_policy_names_the_kernels_actions<M: Rig>() {
    let p = M::intent().policy;
    M::validate_policy(&p).expect("the fixture's policy");
    M::kernel_agrees(&p);
    let refs: Vec<_> = [
        M::OFFER_ACTION,
        M::ACCEPT_ACTION,
        M::INSTALL_ACTION,
        M::CANCEL_ACTION,
    ]
    .into_iter()
    .map(|a| M::action_ref(&p, a).expect(a))
    .collect();
    for (i, r) in refs.iter().enumerate() {
        assert_eq!(r.key().owner.as_str(), "industry", "{r:?}");
        assert!(!refs[..i].contains(r), "four distinct actions");
    }
    for (why, bad) in M::bad_policies(&p) {
        assert!(M::validate_policy(&bad).is_err(), "{why}");
    }
}

/// The subject's fee is bounded by the policy and paid to one of its sites, by someone other
/// than the site; decoding validates.
pub(crate) fn ud4b_the_quote_is_bounded_and_paid_to_its_bay<M: Rig>() {
    let intent = M::intent();
    let (p, s) = (&intent.policy, &intent.subject);
    M::matches_policy(s, p).expect("matches");
    for (why, check, bad) in M::bad_subjects(s) {
        match check {
            Check::Validate => assert!(M::validate_subject(&bad).is_err(), "{why}"),
            Check::Policy => assert!(M::matches_policy(&bad, p).is_err(), "{why}"),
        }
    }
    let encoded = serde_json::to_value(s).expect("encodes");
    assert_eq!(
        &serde_json::from_value::<M::Subject>(encoded).expect("decodes"),
        s
    );
    let mut bad = serde_json::to_value(s).expect("encodes");
    M::bad_wire(&mut bad);
    assert!(
        serde_json::from_value::<M::Subject>(bad).is_err(),
        "decoding validates"
    );
}

/// Accept holds the fee for the order; Install captures exactly it to the provider; Cancel
/// releases it; each under its own key.
pub(crate) fn ud4b_the_plans_hold_capture_and_release_the_quote<M: Rig>() {
    let intent = M::intent();
    let p = &intent.policy;
    let accept = CommitToken::from_bytes([10; 16]);
    let funding = plans::funding(
        &intent,
        accept,
        &M::action_ref(p, M::ACCEPT_ACTION).expect("accept"),
    )
    .expect("a funding plan");
    assert_eq!(funding.idempotency.namespace, M::NAMESPACE);
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
    assert_eq!((*account, amount.minor()), (M::PAYER, M::FEE));
    assert_eq!(
        purpose,
        &crate::HoldPurpose::Reservation {
            kind: M::HOLD_PURPOSE.into(),
            id: 1
        }
    );
    assert!(
        plans::funding(
            &intent,
            accept,
            &M::action_ref(p, M::CANCEL_ACTION).expect("cancel")
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
        &M::action_ref(p, M::INSTALL_ACTION).expect("install"),
        &stamp(7200),
        Some(refit),
    )
    .expect("an install plan");
    let lines: Vec<_> = captured
        .requested_lines
        .iter()
        .map(|l| (l.account, l.delta.minor()))
        .collect();
    assert_eq!(lines, [(M::PAYER, -M::FEE), (M::PROVIDER, M::FEE)]);
    assert!(matches!(
        captured.hold_ops.as_slice(),
        [HoldOp::Capture { hold: h, amount }] if *h == hold && amount.minor() == M::FEE
    ));
    let released = plans::terminal(
        &intent,
        accept,
        hold,
        install,
        &M::action_ref(p, M::CANCEL_ACTION).expect("cancel"),
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
            &M::action_ref(p, M::INSTALL_ACTION).expect("install"),
            &stamp(M::RESERVATION_SECONDS),
            Some(refit),
        )
        .is_err(),
        "no install after the reservation lapses"
    );
}

/// No plan posts under a commit the subject already spent ([`Leg::commit_reused`] and the
/// offer both legs check): not the Accept, and not the Install or Cancel that follows it.
pub(crate) fn ct7_a_spent_commit_posts_no_plan<M: Rig>() {
    let intent = M::intent();
    let p = &intent.policy;
    let accept = CommitToken::from_bytes([10; 16]);
    let accept_action = M::action_ref(p, M::ACCEPT_ACTION).expect("accept");
    let install_action = M::action_ref(p, M::INSTALL_ACTION).expect("install");
    let cancel_action = M::action_ref(p, M::CANCEL_ACTION).expect("cancel");
    let refit = DefinitionDigest::from_bytes([12; 32]);
    plans::funding(&intent, accept, &accept_action).expect("a fresh commit funds");
    let spent = M::spent(&intent.subject);
    assert!(!spent.is_empty());
    for commit in spent {
        assert!(
            plans::funding(&intent, commit, &accept_action).is_err(),
            "{commit:?} funds"
        );
        for (action, part) in [(&install_action, Some(refit)), (&cancel_action, None)] {
            plans::terminal(
                &intent,
                accept,
                HoldId(5),
                CommitToken::from_bytes([11; 16]),
                action,
                &stamp(7200),
                part,
            )
            .expect("a fresh commit ends the order");
            assert!(
                plans::terminal(
                    &intent,
                    accept,
                    HoldId(5),
                    commit,
                    action,
                    &stamp(7200),
                    part
                )
                .is_err(),
                "{commit:?} ends the order"
            );
        }
    }
}

/// A replayed history set in which two orders share what the leg holds exclusive
/// ([`Leg::exclusive`]: the bay's hull while open, the core refit's incoming core once
/// installed) is refused; the same set with the second order elsewhere is accepted.
pub(crate) fn ct7_a_replayed_history_keeps_the_legs_exclusive_object<M: Rig>() {
    let first = M::intent();
    let rival = M::rival(&first);
    let elsewhere = M::elsewhere(&rival);
    let install = !M::EXCLUSIVE_WHILE_OPEN;
    let build = |second: &FundingIntent<M>| {
        let mut audit = ReceiptAudit::<M>::default();
        for (n, intent) in [(1_u8, &first), (2, second)] {
            let h = history(intent, n, install);
            h.validate().expect("each history is valid alone");
            audit.histories.insert(h.funding.commit, h);
        }
        audit
    };
    build(&elsewhere)
        .finish_replay()
        .expect("two orders, two objects");
    assert!(
        build(&rival).finish_replay().is_err(),
        "two orders, one exclusive object"
    );
}

/// The supplier's acknowledged offer is the one the subject takes, seen before the acceptance,
/// inside its five-minute window ([`Leg::supplier_validate`]); an intent with any other is not
/// valid and funds no plan.
pub(crate) fn ct7_the_supplier_attributes_the_offer_taken<M: Rig>() {
    let intent = M::intent();
    let accept = CommitToken::from_bytes([10; 16]);
    let action = M::action_ref(&intent.policy, M::ACCEPT_ACTION).expect("accept");
    intent.validate().expect("the fixture");
    let bad = M::bad_suppliers(&intent);
    assert!(!bad.is_empty());
    for (why, supplier) in bad {
        let other = FundingIntent {
            supplier,
            ..intent.clone()
        };
        assert!(other.validate().is_err(), "{why}");
        assert!(
            plans::funding(&other, accept, &action).is_err(),
            "{why} funds"
        );
    }
}

/// A leg whose composition has no definitions yet is not ready to release a fee
/// ([`Leg::cleanup_ready`], which a Cancel asks first), and its policy is the one it was made
/// with ([`Leg::policy`]).
pub(crate) fn ct7_an_uncomposed_leg_is_not_ready<M: Rig>() {
    let intent = M::intent();
    let config = M::config(&intent);
    assert_eq!(M::policy(&config), &intent.policy);
    assert!(M::cleanup_ready(&config, &mush_core::World::new_minimal()).is_err());
}

/// The seal is the tripwire for an edit that widens it rather than going round it: over the
/// crate's source, exactly two `impl Member for` lines, both in `refit_twin/leg.rs`; exactly
/// two `impl Leg for`, one in each family's `leg.rs`; one `mod members {`, not `pub`; and no
/// `dyn` beside the trait's name. A census of the source, not a proof: the compiler is that
/// (the `compile_fail` example on `Leg`, and the attempts in the cut note).
#[test]
fn the_leg_seal_admits_exactly_two() {
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("a source dir") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    files.sort();
    let member = ["impl ", "Member for "].concat();
    let leg = ["impl ", "Leg for "].concat();
    let dynamic = ["dyn ", "Leg"].concat();
    let module = ["mod ", "members"].concat();
    let (mut members, mut legs, mut modules) = (Vec::new(), Vec::new(), Vec::new());
    for path in &files {
        let name = path
            .strip_prefix(&root)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        for line in std::fs::read_to_string(path).expect("a source").lines() {
            let line = line.trim();
            if line.starts_with(&member) {
                members.push(name.clone());
            }
            if line.starts_with(&leg) {
                legs.push(name.clone());
            }
            if line.contains(&module) && !line.starts_with("//") {
                modules.push(line.to_owned());
            }
            assert!(
                !line.contains(&dynamic) || line.contains(&[dynamic.as_str(), "_"].concat()),
                "{name}: {line}"
            );
        }
    }
    assert_eq!(members, ["refit_twin/leg.rs", "refit_twin/leg.rs"]);
    assert_eq!(legs, ["bay_refit/leg.rs", "core_refit/leg.rs"]);
    assert_eq!(modules, [[module.as_str(), " {"].concat()]);
}

/// A funded history, and with `install` its Install, built from the leg's own plans the way
/// the fold builds it from the journal.
pub(crate) fn history<M: Rig>(
    intent: &FundingIntent<M>,
    n: u8,
    install: bool,
) -> PaymentHistory<M> {
    let accept = CommitToken::from_bytes([10 + n; 16]);
    let action = M::action_ref(&intent.policy, M::ACCEPT_ACTION).expect("accept");
    let plan = plans::funding(intent, accept, &action).expect("a funding plan");
    let hold = Hold {
        id: HoldId(u64::from(n)),
        account: intent.subject.payer(),
        amount: intent.price().expect("a price"),
        placed_by: plan.actor.clone(),
        beneficiary: Some(PrincipalId::System(
            super::Policy::issuer(&intent.policy).clone(),
        )),
        holder: plan.idempotency.clone(),
        purpose: HoldPurpose::Reservation {
            kind: M::HOLD_PURPOSE.into(),
            id: intent.subject.order(),
        },
        expires: None,
        status: HoldStatus::Held,
        revision: 1,
        last_journal_seq: 10 * u64::from(n),
    };
    let mut h = PaymentHistory {
        intent: Box::new(intent.clone()),
        funding: Box::new(FundingReceipt {
            commit: accept,
            action,
            transaction: TransactionId(10 * u64::from(n)),
            journal_seq: 10 * u64::from(n),
            posted_at: 200,
            plan_hash: crate::hash::plan_hash(&plan),
            hold: hold.clone(),
        }),
        terminal: None,
    };
    if install {
        let at = stamp(7200);
        let part = DefinitionDigest::from_bytes([30 + n; 32]);
        let commit = CommitToken::from_bytes([20 + n; 16]);
        let action = M::action_ref(&intent.policy, M::INSTALL_ACTION).expect("install");
        let plan: EconomyPlan =
            plans::terminal(intent, accept, hold.id, commit, &action, &at, Some(part))
                .expect("an install plan");
        let transaction = TransactionId(10 * u64::from(n) + 1);
        h.terminal = Some(Box::new(TerminalReceipt {
            commit,
            action,
            transaction,
            journal_seq: 10 * u64::from(n) + 1,
            posted_at: 300,
            at,
            plan_hash: crate::hash::plan_hash(&plan),
            hold: Hold {
                status: HoldStatus::Consumed {
                    transaction,
                    amount: intent.price().expect("a price"),
                },
                revision: 2,
                last_journal_seq: 10 * u64::from(n) + 1,
                ..hold
            },
            result: M::installed(part),
        }));
    }
    h
}
