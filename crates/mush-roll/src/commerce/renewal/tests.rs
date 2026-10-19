//! Structural history tests. These do not fabricate an accepted E4 grant;
//! actual owner/output/audit integration must also pass at the paired boundary.
use super::super::{
    CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow,
    WorldIcSeconds,
    entitlement::{
        DownstreamTermsV1, FiledRoyaltyTermsV1, GrantEconomicsV1, PaidTermEconomicsV1,
        RenewalTemplateV1,
    },
    paid::{PaidForm, PaidTermRecord, PaidTerms, PersonalPaidLicence},
    rating::RatingClass,
    records::{Record, RevisionPolicy},
};
use super::records::*;
use super::*;
use mush_core::{
    ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
};
use mush_economy::{
    Amount, InstrumentId,
    managed_renewal::{RenewalFundingReceiptV1, RenewalTerminalReceiptV1, UnfundedCloseReason},
};
use serde_json::json;
use std::collections::BTreeMap;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn token(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn digest(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn stamp(at: i64) -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at,
    }
}
fn definition(kind: &str, id: &str) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("market")?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            1,
            json!({"format":1}),
        )?,
    )?)
}
fn identity() -> TestResult<RevisionIdentity> {
    Ok(serde_json::from_value(
        json!({"format":1,"revision":{"key":{"character":1,"mind":1,"number":1},
        "digest":digest(1)},"source":digest(2)}),
    )?)
}
struct Birth {
    licence: PersonalPaidLicence,
    term: PaidTermRecord,
    economics: PaidTermEconomicsV1,
}
fn birth() -> TestResult<Birth> {
    let id = LicenceId::new(7)?;
    let one = TermGeneration::new(1)?;
    let terms = PaidTerms::original_personal_form(RatingClass::I, PaidForm::OpenResale {}, false)?;
    let policy = RevisionPolicy::Pin {
        filed: FiledRevisionId::new(3)?,
        revision: identity()?,
    };
    let chain =
        FiledRoyaltyTermsV1::capture(FiledRevisionId::new(3)?, identity()?, token(2), Vec::new())?;
    let grant = GrantEconomicsV1::new(
        DownstreamTermsV1::Source {
            royalty_permille: 73,
        },
        RenewalTemplateV1::new(
            InstrumentId::new("cr").ok_or("instrument")?,
            Amount::from_minor(500),
            100,
            20,
            terms,
            policy,
            definition("entitlement_policy", "personal")?,
        )?,
        chain.reference()?,
    )?;
    Ok(Birth {
        licence: PersonalPaidLicence::original(
            id,
            CharterId::new(9)?,
            LineId::new(2)?,
            ObjId(5),
            one,
            token(1),
        )?,
        term: PaidTermRecord::original(
            id,
            one,
            TermWindow::new(
                WorldIcSeconds::from_stamp(&stamp(10))?,
                WorldIcSeconds::from_stamp(&stamp(10))?,
                WorldIcSeconds::from_stamp(&stamp(110))?,
                WorldIcSeconds::from_stamp(&stamp(130))?,
            )?,
            terms,
            policy,
            token(1),
        )?,
        economics: PaidTermEconomicsV1::capture(id, one, token(1), digest(9), grant)?,
    })
}
fn draft(b: &Birth, n: u8, at: i64, attempt: RenewalAttemptV1) -> TestResult<RenewalOperationV1> {
    let row = RenewalOperationV1 {
        format: 1,
        operation: token(n),
        action: definition("action", "draft_mind_renewal")?,
        attempt,
        binding: RenewalBindingV1::capture(&b.licence, &b.term, &b.economics)?,
        offer_digest: digest(n),
        contract: serde_json::from_value(json!(n))?,
        consent_hash: [n; 32],
        created_at_ic: stamp(at),
        created_at_utc: at,
        state: RenewalStateV1::Drafted {},
    };
    row.validate()?;
    Ok(row)
}
fn funded(row: &RenewalOperationV1) -> TestResult<RenewalOperationV1> {
    let n = u8::try_from(row.created_at_utc)?;
    let receipt: RenewalFundingReceiptV1 = serde_json::from_value(
        json!({"format":1,"commit":token(n+1),
        "action":definition("action","fund_mind_renewal")?,"contract":row.contract,"hold":u64::from(n),
        "transaction":u64::from(n),"journal_seq":u64::from(n),"consent_hash":row.consent_hash,
        "at_ic":stamp(i64::from(n)+1),"posted_at_utc":i64::from(n)+1}),
    )?;
    let next = RenewalOperationV1 {
        state: RenewalStateV1::Funded { funding: receipt },
        ..row.clone()
    };
    next.follows(row)?;
    Ok(next)
}
fn terminal(row: &RenewalOperationV1, grant: bool) -> TestResult<RenewalOperationV1> {
    let funding = row.state.funding().ok_or("funding")?.clone();
    let n = u8::try_from(row.created_at_utc)?;
    let at = stamp(i64::from(n) + 2);
    let receipt: RenewalTerminalReceiptV1 = serde_json::from_value(
        json!({"format":1,"commit":token(n+2),
        "action":definition("action",if grant{"settle_mind_renewal"}else{"refund_mind_renewal"})?,
        "outcome":if grant{"granted"}else{"refunded"},"transaction":u64::from(n)+1,"journal_seq":u64::from(n)+1,
        "at_ic":at,"posted_at_utc":i64::from(n)+2,"issued_at_ic":if grant{Some(at)}else{None}}),
    )?;
    let state = if grant {
        RenewalStateV1::Granted {
            funding,
            terminal: receipt,
        }
    } else {
        RenewalStateV1::Refunded {
            funding,
            terminal: receipt,
        }
    };
    let next = RenewalOperationV1 {
        state,
        ..row.clone()
    };
    next.follows(row)?;
    Ok(next)
}
fn closed(row: &RenewalOperationV1) -> TestResult<RenewalOperationV1> {
    let n = u8::try_from(row.created_at_utc)?;
    let next = RenewalOperationV1 {
        state: RenewalStateV1::UnfundedClosed {
            closure: RenewalClosureV1 {
                commit: token(n + 2),
                action: definition("action", "close_mind_renewal")?,
                at_ic: stamp(i64::from(n) + 2),
                posted_at_utc: i64::from(n) + 2,
                reason: UnfundedCloseReason::Declined,
            },
        },
        ..row.clone()
    };
    next.follows(row)?;
    Ok(next)
}
fn granted_birth(row: &RenewalOperationV1) -> TestResult<Birth> {
    let (licence, term, economics) = row.granted_rows()?;
    Ok(Birth {
        licence,
        term,
        economics,
    })
}

#[test]
fn strict_rows_reject_missing_null_duplicate_and_nested_unknown_fields() -> TestResult {
    let row = draft(&birth()?, 10, 20, RenewalAttemptV1::Automatic {})?;
    let raw = serde_json::to_vec(&row)?;
    assert_eq!(RenewalOperationV1::decode(&raw)?, row);
    let repeated = String::from_utf8(raw)?.replacen("{", "{\"format\":1,", 1);
    assert!(RenewalOperationV1::decode(repeated.as_bytes()).is_err());
    let mut value = serde_json::to_value(&row)?;
    value
        .get_mut("state")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("state")?
        .insert("allowed".into(), json!(true));
    assert!(serde_json::from_value::<RenewalOperationV1>(value).is_err());
    let head = RenewalHeadV1::for_operation(&closed(&row)?)?;
    let mut value = serde_json::to_value(&head)?;
    assert_eq!(value.get("pending"), Some(&serde_json::Value::Null));
    value.as_object_mut().ok_or("head")?.remove("pending");
    assert!(serde_json::from_value::<RenewalHeadV1>(value).is_err());
    assert!(
        serde_json::from_str::<RenewalAttemptV1>(
            r#"{"kind":"automatic","previous_operation":null}"#
        )
        .is_err()
    );
    assert!(RenewalOperationV1::decode(&vec![b' '; MAX_OPERATION_BYTES + 1]).is_err());
    Ok(())
}

#[test]
fn closed_attempt_can_precede_exact_retry_grant_without_eternal_next_absence() -> TestResult {
    let b = birth()?;
    let closed = closed(&draft(&b, 90, 20, RenewalAttemptV1::Automatic {})?)?;
    let retry = draft(
        &b,
        10,
        40,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: closed.operation,
        },
    )?;
    chain::check_draft(
        &b.licence,
        &retry,
        Some(&RenewalHeadV1::for_operation(&closed)?),
        Some(&closed),
    )?;
    let granted = terminal(&funded(&retry)?, true)?;
    let current = granted_birth(&granted)?;
    let terms = BTreeMap::from([
        (b.term.generation(), &b.term),
        (current.term.generation(), &current.term),
    ]);
    let economics = BTreeMap::from([
        (b.economics.generation(), &b.economics),
        (current.economics.generation(), &current.economics),
    ]);
    // Lexical token order is deliberately the opposite of causal order.
    chain::verify(
        &current.licence,
        Some(&RenewalHeadV1::for_operation(&granted)?),
        &[&granted, &closed],
        &terms,
        &economics,
    )?;
    assert!(
        chain::verify(
            &current.licence,
            Some(&RenewalHeadV1::for_operation(&granted)?),
            &[&granted],
            &terms,
            &economics
        )
        .is_err()
    );
    let mut wrong = current.term.clone();
    let mut wire = serde_json::to_value(&wrong)?;
    wire.as_object_mut()
        .ok_or("term")?
        .insert("operation".into(), json!(closed.operation));
    wrong = serde_json::from_value(wire)?;
    let wrong_terms =
        BTreeMap::from([(b.term.generation(), &b.term), (wrong.generation(), &wrong)]);
    assert!(
        chain::verify(
            &current.licence,
            Some(&RenewalHeadV1::for_operation(&granted)?),
            &[&closed, &granted],
            &wrong_terms,
            &economics
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn retry_chain_rejects_branch_orphan_wrong_holder_and_automatic_repeat() -> TestResult {
    let b = birth()?;
    let refund = terminal(
        &funded(&draft(&b, 80, 20, RenewalAttemptV1::Automatic {})?)?,
        false,
    )?;
    let head = RenewalHeadV1::for_operation(&refund)?;
    let retry = draft(
        &b,
        10,
        40,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: refund.operation,
        },
    )?;
    let branch = draft(&b, 11, 50, retry.attempt)?;
    let terms = BTreeMap::from([(b.term.generation(), &b.term)]);
    let economics = BTreeMap::from([(b.economics.generation(), &b.economics)]);
    chain::verify(
        &b.licence,
        Some(&RenewalHeadV1::for_operation(&retry)?),
        &[&retry, &refund],
        &terms,
        &economics,
    )?;
    assert!(
        chain::verify(
            &b.licence,
            Some(&head),
            &[&refund, &retry, &branch],
            &terms,
            &economics
        )
        .is_err()
    );
    assert!(
        chain::check_draft(
            &b.licence,
            &draft(&b, 12, 40, RenewalAttemptV1::Automatic {})?,
            Some(&head),
            Some(&refund)
        )
        .is_err()
    );
    let mut wrong = retry.clone();
    wrong.binding.holder = ObjId(6);
    assert!(chain::check_draft(&b.licence, &wrong, Some(&head), Some(&refund)).is_err());
    let mut orphan = retry;
    orphan.attempt = RenewalAttemptV1::BuyerRetry {
        previous_operation: token(99),
    };
    assert!(
        chain::verify(
            &b.licence,
            Some(&RenewalHeadV1::for_operation(&orphan)?),
            &[&refund, &orphan],
            &terms,
            &economics
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn current_generation_advances_without_rewriting_birth_or_historical_acquisition() -> TestResult {
    let b = birth()?;
    let first = terminal(
        &funded(&draft(&b, 80, 20, RenewalAttemptV1::Automatic {})?)?,
        true,
    )?;
    let second_birth = granted_birth(&first)?;
    let second = terminal(
        &funded(&draft(
            &second_birth,
            10,
            40,
            RenewalAttemptV1::Automatic {},
        )?)?,
        true,
    )?;
    let current = granted_birth(&second)?;
    let terms = BTreeMap::from([
        (b.term.generation(), &b.term),
        (second_birth.term.generation(), &second_birth.term),
        (current.term.generation(), &current.term),
    ]);
    let economics = BTreeMap::from([
        (b.economics.generation(), &b.economics),
        (second_birth.economics.generation(), &second_birth.economics),
        (current.economics.generation(), &current.economics),
    ]);
    chain::verify(
        &current.licence,
        Some(&RenewalHeadV1::for_operation(&second)?),
        &[&second, &first],
        &terms,
        &economics,
    )?;
    assert_eq!(current.licence.operation(), b.licence.operation());
    assert_eq!(current.term.operation(), second.operation);
    first
        .binding
        .check_predecessor(&current.licence, &b.term, &b.economics)?;
    second.binding.check_predecessor(
        &current.licence,
        &second_birth.term,
        &second_birth.economics,
    )?;
    assert!(b.term.matches_current(&current.licence).is_err());
    assert!(
        chain::verify(
            &current.licence,
            None,
            &[&first, &second],
            &terms,
            &economics
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn pending_funding_preserves_old_current_rows_and_blocks_new_attempt() -> TestResult {
    let b = birth()?;
    let old = (
        serde_json::to_vec(&b.licence)?,
        serde_json::to_vec(&b.term)?,
        serde_json::to_vec(&b.economics)?,
    );
    let draft = draft(&b, 80, 20, RenewalAttemptV1::Automatic {})?;
    let funded = funded(&draft)?;
    let head = RenewalHeadV1::for_operation(&funded)?;
    assert_eq!(head, RenewalHeadV1::for_operation(&draft)?);
    chain::verify(
        &b.licence,
        Some(&head),
        &[&funded],
        &BTreeMap::from([(b.term.generation(), &b.term)]),
        &BTreeMap::from([(b.economics.generation(), &b.economics)]),
    )?;
    assert!(funded.granted_rows().is_err());
    assert!(chain::check_draft(&b.licence, &draft, Some(&head), Some(&funded)).is_err());
    assert!(
        old == (
            serde_json::to_vec(&b.licence)?,
            serde_json::to_vec(&b.term)?,
            serde_json::to_vec(&b.economics)?
        ),
        "pending renewal rewrote issued rows"
    );
    Ok(())
}

#[test]
fn exact_transition_and_issued_clock_refuse_crossed_receipts() -> TestResult {
    let b = birth()?;
    let draft = draft(&b, 80, 20, RenewalAttemptV1::Automatic {})?;
    let funded = funded(&draft)?;
    let granted = terminal(&funded, true)?;
    assert!(granted.follows(&draft).is_err());
    assert!(granted.follows(&granted).is_err());
    let mut crossed = granted.clone();
    crossed.offer_digest = digest(99);
    assert!(crossed.follows(&funded).is_err());
    let mut wire = serde_json::to_value(&granted)?;
    let terminal = wire
        .get_mut("state")
        .and_then(|s| s.get_mut("terminal"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("terminal")?;
    let other = ClockStamp {
        clock: ClockRef::WorldIc { generation: 2 },
        at: 22,
    };
    terminal.insert("at_ic".into(), json!(other));
    terminal.insert("issued_at_ic".into(), json!(other));
    assert!(serde_json::from_value::<RenewalOperationV1>(wire).is_err());
    let refund = terminal_refund_clock(&funded, 2)?;
    refund.follows(&funded)?;
    assert!(refund.granted_rows().is_err());
    Ok(())
}
fn terminal_refund_clock(
    funded: &RenewalOperationV1,
    generation: u64,
) -> TestResult<RenewalOperationV1> {
    let refund = terminal(funded, false)?;
    let mut wire = serde_json::to_value(&refund)?;
    let terminal = wire
        .get_mut("state")
        .and_then(|s| s.get_mut("terminal"))
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("terminal")?;
    terminal.insert(
        "at_ic".into(),
        json!(ClockStamp {
            clock: ClockRef::WorldIc { generation },
            at: 22
        }),
    );
    Ok(serde_json::from_value(wire)?)
}

#[test]
fn exhausted_generation_refuses_before_any_new_offer_or_money() -> TestResult {
    let b = birth()?;
    let max = TermGeneration::new(u64::MAX)?;
    let licence = PersonalPaidLicence::original(
        b.licence.id(),
        b.licence.charter(),
        b.licence.line(),
        b.licence.holder(),
        max,
        b.licence.operation(),
    )?;
    let term = PaidTermRecord::original(
        licence.id(),
        max,
        b.term.window(),
        b.term.terms(),
        b.term.policy(),
        token(9),
    )?;
    let economics = PaidTermEconomicsV1::capture(
        licence.id(),
        max,
        token(9),
        b.economics.product_digest(),
        b.economics.grant().clone(),
    )?;
    assert!(RenewalBindingV1::capture(&licence, &term, &economics).is_err());
    Ok(())
}

#[path = "capacity_tests.rs"]
mod capacity_tests;
