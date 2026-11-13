//! Doc 46 CT.0: the bay refit fee's persisted shapes, on this family's own
//! fixture (`policy()`, `subject()`, `intent()`).
//!
//! The live Accept/Install/Cancel run only inside a core-minted owner
//! preparation (an `OwnerPrepareContext`, which Economy cannot make in its own
//! tests; Industry's bay rig drives it). So this pin stops at the closed money
//! alphabet the owner posts: the three fixed plans (key, actor, cause,
//! correlation, lines, hold op, plan hash), and the receipts and history the
//! fold keeps, built from those plans and checked by the history's own
//! `validate` (which recomputes every plan and hold from the intent).
use super::super::receipt::{BayFundingReceipt, BayPaymentHistory, BayTerminalReceipt};
use super::super::*;
use super::{intent, stamp};
use crate::plan::EconomyPlan;
use crate::record::{Hold, HoldPurpose, HoldStatus};
use crate::{HoldId, TransactionId};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest};
use mush_org::PrincipalId;
use mush_testkit::snapshot::Snapshot;

type Outcome<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn plan_lines(s: &mut Snapshot, plan: &EconomyPlan) -> Outcome {
    s.wire("idempotency", &plan.idempotency)?;
    s.wire("actor", &plan.actor)?;
    s.wire("cause", &plan.cause)?;
    s.line("description", &plan.description);
    for line in &plan.requested_lines {
        s.line(
            "requested line",
            &format!("{} {} {}", line.account, line.delta.minor(), line.memo),
        );
    }
    s.wire("hold ops", &plan.hold_ops)?;
    s.hex("plan hash", &crate::hash::plan_hash(plan));
    if let crate::record::EconomicCause::Ruleset { correlation, .. } = &plan.cause {
        let proof = plans::decode_correlation(correlation)?;
        s.codec_shape("Proof", &proof)?;
    }
    Ok(())
}

#[test]
fn ct0_refit_bay_snapshot() -> Outcome {
    let intent = intent();
    let policy = &intent.policy;
    let subject = &intent.subject;
    let mut s = Snapshot::new();
    s.section("identities");
    for (label, value) in [
        ("offer action", OFFER_ACTION),
        ("accept action", ACCEPT_ACTION),
        ("install action", INSTALL_ACTION),
        ("cancel action", CANCEL_ACTION),
        ("namespace", NAMESPACE),
        ("hold purpose", HOLD_PURPOSE),
        ("operator capability", OPERATOR_CAPABILITY),
    ] {
        s.line(label, value);
    }
    s.line("reservation seconds", &RESERVATION_SECONDS.to_string());
    for (label, value) in [
        ("max orders", MAX_ORDERS),
        ("subject bytes", SUBJECT_BYTES),
        ("history bytes", HISTORY_BYTES),
        ("correlation bytes", plans::CORRELATION_BYTES),
    ] {
        s.line(label, &value.to_string());
    }
    for error in [
        BayRefitError::Invalid,
        BayRefitError::Integrity,
        BayRefitError::Incomplete,
        BayRefitError::Unavailable,
        BayRefitError::Capacity,
    ] {
        s.line("error", &error.to_string());
    }
    s.line("refused", &refused().to_string());

    s.section("policy");
    s.wire("policy", policy)?;
    s.wire("policy digest", &policy.digest()?)?;
    s.wire("industry policy", &policy.industry_policy())?;
    for action in [OFFER_ACTION, ACCEPT_ACTION, INSTALL_ACTION, CANCEL_ACTION] {
        s.wire(&format!("action {action}"), &policy.action_ref(action)?)?;
    }
    s.section("subject");
    s.wire("subject", subject)?;
    s.wire("subject digest", &subject.digest()?)?;
    s.wire("intent", &intent)?;

    let accept = CommitToken::from_bytes([10; 16]);
    let accept_action = policy.action_ref(ACCEPT_ACTION)?;
    let funding_plan = plans::funding(&intent, accept, &accept_action)?;
    s.section("accept | plan");
    plan_lines(&mut s, &funding_plan)?;
    let hold = Hold {
        id: HoldId(5),
        account: subject.payer,
        amount: subject.price(),
        beneficiary: Some(PrincipalId::System(policy.issuer.clone())),
        purpose: HoldPurpose::Reservation {
            kind: HOLD_PURPOSE.into(),
            id: subject.order,
        },
        placed_by: funding_plan.actor.clone(),
        holder: funding_plan.idempotency.clone(),
        expires: None,
        status: HoldStatus::Held,
        last_journal_seq: 1,
        revision: 1,
    };
    let funding = BayFundingReceipt {
        commit: accept,
        action: accept_action,
        transaction: TransactionId(1),
        journal_seq: 1,
        posted_at: 200,
        plan_hash: crate::hash::plan_hash(&funding_plan),
        hold: hold.clone(),
    };
    let funded = BayPaymentHistory {
        intent: Box::new(intent.clone()),
        funding: Box::new(funding.clone()),
        terminal: None,
    };
    funded.validate()?;
    funded.require_terminal_room()?;
    s.section("accept | funded history");
    s.wire("history", &funded)?;

    let mut terminals = Vec::new();
    for (title, action, refit) in [
        (
            "install",
            INSTALL_ACTION,
            Some(DefinitionDigest::from_bytes([12; 32])),
        ),
        ("cancel", CANCEL_ACTION, None),
    ] {
        let commit = CommitToken::from_bytes([11; 16]);
        let at: ClockStamp = stamp(7200);
        let reference = policy.action_ref(action)?;
        let plan = plans::terminal(&intent, accept, hold.id, commit, &reference, &at, refit)?;
        s.section(&format!("{title} | plan"));
        plan_lines(&mut s, &plan)?;
        let mut after = hold.clone();
        after.status = match refit {
            Some(_) => HoldStatus::Consumed {
                transaction: TransactionId(2),
                amount: subject.price(),
            },
            None => HoldStatus::Released,
        };
        after.revision = 2;
        after.last_journal_seq = 2;
        let terminal = BayTerminalReceipt {
            commit,
            action: reference,
            transaction: TransactionId(2),
            journal_seq: 2,
            posted_at: 300,
            at,
            plan_hash: crate::hash::plan_hash(&plan),
            hold: after,
            result: match refit {
                Some(refit) => receipt::BayTerminalResult::Installed { refit },
                None => receipt::BayTerminalResult::Cancelled {},
            },
        };
        let history = BayPaymentHistory {
            terminal: Some(Box::new(terminal.clone())),
            ..funded.clone()
        };
        history.validate()?;
        s.section(&format!("{title} | terminal history"));
        s.wire("history", &history)?;
        terminals.push((terminal, history));
    }

    s.codec_shape("BaySite", policy.bays.first().ok_or("a bay")?)?;
    s.codec_shape("BayPaymentPolicy", policy)?;
    s.codec("BaySubject", subject, |json| json["fee_minor"] = 0.into())?;
    s.codec_shape("BayOfferAttribution", &intent.supplier)?;
    s.codec_shape("FundingIntent", &intent)?;
    s.codec_shape("BayFundingReceipt", &funding)?;
    s.codec_shape("BayPaymentHistory", &funded)?;
    for (terminal, history) in &terminals {
        s.codec_shape("BayTerminalResult", &terminal.result)?;
        s.codec_shape("BayTerminalReceipt", terminal)?;
        s.codec_shape("BayPaymentHistory", history)?;
    }
    s.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/refit_bay.snap")?;
    Ok(())
}
