#![cfg(test)]

//! Doc 46 CT.0: the certified core fitting's persisted shapes.
//!
//! As for its bay sibling (`bay_refit/tests/ct0_snapshot.rs`), the live
//! Accept/Install/Cancel run only inside a core-minted owner preparation that
//! Economy's own tests cannot make, so this pin stops at the closed money
//! alphabet: the three fixed plans and the receipts and history the fold
//! keeps, built from those plans and checked by the history's own `validate`.
//! Two subjects: a chartered `core_refit` (its `kind` written by absence, a
//! displaced core present) and a `core_ungovern` (the `kind` present, no
//! displaced core), under the M8b composed policy; and the line-only policy,
//! whose optional keys are absent.
use super::plans::{self, FundingIntent};
use super::receipt::{RefitFundingReceipt, RefitPaymentHistory, RefitTerminalReceipt};
use super::*;
use crate::core_integration::{CoreClass, ScopedCoreAccount};
use crate::plan::EconomyPlan;
use crate::record::{Hold, HoldPurpose, HoldStatus};
use crate::{HoldId, TransactionId};
use mush_core::{
    DefinitionDescriptor, DefinitionDigest, DefinitionId, DefinitionKind, DefinitionRef,
    DefinitionRegistryBuilder, ObjId, OwnerId,
};
use mush_org::{OrgId, OrgScopeId, PrincipalId, SystemPrincipalId};
use mush_testkit::fixtures::stamp;
use mush_testkit::snapshot::Snapshot;

type Outcome<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn reference(owner: &str, kind: &str, id: &str) -> Outcome<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new(owner)?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            1,
            serde_json::json!({ "fixture": id }),
        )?,
    )?)
}
fn family(prefix: &str) -> Outcome<[DefinitionRef; 3]> {
    Ok([
        reference("tacsim", "unit", &format!("{prefix}_i"))?,
        reference("tacsim", "unit", &format!("{prefix}_ii"))?,
        reference("tacsim", "unit", &format!("{prefix}_iii"))?,
    ])
}
fn account(n: u64) -> ScopedCoreAccount {
    ScopedCoreAccount {
        account: crate::AccountId(n),
        organization: OrgId(n),
        scope: OrgScopeId(n),
    }
}
/// The M8b fixtures (`subject.rs`'s `m8b_tests`): the line-only policy and
/// the composed one with a second family, the ungoverning terms and a
/// coreless base.
fn line_only() -> Outcome<RefitPaymentPolicy> {
    Ok(RefitPaymentPolicy {
        issuer: SystemPrincipalId::new("hardware.factory").ok_or("an issuer")?,
        instrument: crate::InstrumentId::new("tacsim.credit").ok_or("an instrument")?,
        provider: account(1),
        factory: ObjId(10),
        recipe: reference("tacsim", "recipe", "core_refit")?,
        fitted_units: family("line")?,
        families: Vec::new(),
        ungovern: None,
        coreless: Vec::new(),
    })
}
fn composed() -> Outcome<RefitPaymentPolicy> {
    Ok(RefitPaymentPolicy {
        families: vec![family("rig")?],
        ungovern: Some(UngovernTerms {
            provider: account(2),
            factory: ObjId(20),
            recipe: reference("tacsim", "recipe", "core_ungovern")?,
            fee_minor: 25_000,
            twins: vec![family("line_twin")?, family("rig_twin")?],
        }),
        coreless: vec![CorelessBase {
            base: reference("tacsim", "unit", "rig_base")?,
            family: 1,
        }],
        ..line_only()?
    })
}
fn intent(
    policy: &RefitPaymentPolicy,
    kind: RefitKind,
    target: DefinitionRef,
    displaced: Option<ObjId>,
) -> Outcome<FundingIntent> {
    let subject = RefitSubject {
        order: 1,
        intent: DefinitionDigest::from_bytes([1; 32]),
        offer: CommitToken::from_bytes([2; 16]),
        offer_digest: DefinitionDigest::from_bytes([3; 32]),
        requester: ObjId(7),
        payer: crate::AccountId(9),
        core_acceptance: CommitToken::from_bytes([4; 16]),
        incoming: ObjId(50),
        hull: ObjId(99),
        unit_revision: 3,
        target,
        class: CoreClass::II,
        displaced,
        policy: policy.digest()?,
        accepted_at: stamp(0),
        expires_at: stamp(RESERVATION_SECONDS),
        kind,
    };
    let intent = FundingIntent {
        supplier: RefitOfferAttribution {
            operation: subject.offer,
            digest: subject.offer_digest,
            action: policy.action_ref(OFFER_ACTION)?,
            operator: ObjId(8),
            observed_ic: stamp(0),
            observed_utc: 100,
            expires_utc: 400,
        },
        subject,
        policy: policy.clone(),
    };
    intent.validate()?;
    Ok(intent)
}
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
/// Accept, then Install and Cancel, each from the accepted history.
fn walk(s: &mut Snapshot, title: &str, intent: &FundingIntent) -> Outcome {
    let policy = &intent.policy;
    let subject = &intent.subject;
    s.section(&format!("{title} | subject"));
    s.wire("subject", subject)?;
    s.wire("subject digest", &subject.digest()?)?;
    s.line("price", &intent.price()?.minor().to_string());
    s.wire("intent", intent)?;
    s.codec("RefitSubject", subject, |json| {
        json["incoming"] = json["hull"].clone();
    })?;
    s.codec_shape("RefitOfferAttribution", &intent.supplier)?;
    s.codec_shape("FundingIntent", intent)?;

    let accept = CommitToken::from_bytes([10; 16]);
    let accept_action = policy.action_ref(ACCEPT_ACTION)?;
    let funding_plan = plans::funding(intent, accept, &accept_action)?;
    s.section(&format!("{title} | accept plan"));
    plan_lines(s, &funding_plan)?;
    let hold = Hold {
        id: HoldId(5),
        account: subject.payer,
        amount: intent.price()?,
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
    let funding = RefitFundingReceipt {
        commit: accept,
        action: accept_action,
        transaction: TransactionId(1),
        journal_seq: 1,
        posted_at: 200,
        plan_hash: crate::hash::plan_hash(&funding_plan),
        hold: hold.clone(),
    };
    let funded = RefitPaymentHistory {
        intent: Box::new(intent.clone()),
        funding: Box::new(funding.clone()),
        terminal: None,
    };
    funded.validate()?;
    funded.require_terminal_room()?;
    s.section(&format!("{title} | funded history"));
    s.wire("history", &funded)?;
    s.codec_shape("RefitFundingReceipt", &funding)?;
    s.codec_shape("RefitPaymentHistory", &funded)?;
    for (phase, action, replacement) in [
        (
            "install",
            INSTALL_ACTION,
            Some(DefinitionDigest::from_bytes([12; 32])),
        ),
        ("cancel", CANCEL_ACTION, None),
    ] {
        let commit = CommitToken::from_bytes([11; 16]);
        let at = stamp(7200);
        let reference = policy.action_ref(action)?;
        let plan = plans::terminal(
            intent,
            accept,
            hold.id,
            commit,
            &reference,
            &at,
            replacement,
        )?;
        s.section(&format!("{title} | {phase} plan"));
        plan_lines(s, &plan)?;
        let mut after = hold.clone();
        after.status = match replacement {
            Some(_) => HoldStatus::Consumed {
                transaction: TransactionId(2),
                amount: intent.price()?,
            },
            None => HoldStatus::Released,
        };
        after.revision = 2;
        after.last_journal_seq = 2;
        let terminal = RefitTerminalReceipt {
            commit,
            action: reference,
            transaction: TransactionId(2),
            journal_seq: 2,
            posted_at: 300,
            at,
            plan_hash: crate::hash::plan_hash(&plan),
            hold: after,
            result: match replacement {
                Some(replacement) => RefitTerminalResult::Installed { replacement },
                None => RefitTerminalResult::Cancelled {},
            },
        };
        let history = RefitPaymentHistory {
            terminal: Some(Box::new(terminal.clone())),
            ..funded.clone()
        };
        history.validate()?;
        s.section(&format!("{title} | {phase} history"));
        s.wire("history", &history)?;
        s.codec_shape("RefitTerminalResult", &terminal.result)?;
        s.codec_shape("RefitTerminalReceipt", &terminal)?;
        s.codec_shape("RefitPaymentHistory", &history)?;
    }
    Ok(())
}

#[test]
fn ct0_refit_core_snapshot() -> Outcome {
    let mut s = Snapshot::new();
    s.section("identities");
    for (label, value) in [
        ("offer action", OFFER_ACTION),
        ("accept action", ACCEPT_ACTION),
        ("install action", INSTALL_ACTION),
        ("cancel action", CANCEL_ACTION),
        ("namespace", NAMESPACE),
        ("hold purpose", HOLD_PURPOSE),
    ] {
        s.line(label, value);
    }
    for (label, value) in [
        ("fitting minor", FITTING_MINOR),
        ("reservation seconds", RESERVATION_SECONDS),
    ] {
        s.line(label, &value.to_string());
    }
    for (label, value) in [
        ("max orders", MAX_ORDERS),
        ("subject bytes", SUBJECT_BYTES),
        ("history bytes", HISTORY_BYTES),
        ("correlation bytes", plans::CORRELATION_BYTES),
    ] {
        s.line(label, &value.to_string());
    }
    for error in [
        RefitError::Invalid,
        RefitError::Integrity,
        RefitError::Incomplete,
        RefitError::Unavailable,
        RefitError::Capacity,
    ] {
        s.line("error", &error.to_string());
    }
    s.line("refused", &refused().to_string());
    for kind in [RefitKind::CoreRefit, RefitKind::CoreUngovern] {
        s.wire("kind", &kind)?;
    }

    let line = line_only()?;
    let policy = composed()?;
    for (title, p) in [("line-only policy", &line), ("composed policy", &policy)] {
        s.section(title);
        s.wire("policy", p)?;
        s.wire("policy digest", &p.digest()?)?;
        for action in [OFFER_ACTION, ACCEPT_ACTION, INSTALL_ACTION, CANCEL_ACTION] {
            s.wire(&format!("action {action}"), &p.action_ref(action)?)?;
        }
        s.codec_shape("RefitPaymentPolicy", p)?;
    }
    let ungovern = policy.ungovern.as_ref().ok_or("ungoverning terms")?;
    s.codec_shape("UngovernTerms", ungovern)?;
    s.codec_shape("CorelessBase", policy.coreless.first().ok_or("a base")?)?;

    let refit = intent(
        &policy,
        RefitKind::CoreRefit,
        family("line")?[1].clone(),
        Some(ObjId(51)),
    )?;
    walk(&mut s, "core refit", &refit)?;
    let twin = intent(
        &policy,
        RefitKind::CoreUngovern,
        family("line_twin")?[1].clone(),
        None,
    )?;
    walk(&mut s, "core ungovern", &twin)?;
    s.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/refit_core.snap")?;
    Ok(())
}
