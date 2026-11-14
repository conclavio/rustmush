//! Structural history tests. These do not fabricate an accepted E4 grant;
//! actual owner/output/audit integration must also pass at the paired boundary.
//!
//! Doc 46 CT.5: each scenario is one body, generic over the issuer, and each family's test
//! module runs it under its old name (`instantiate!`). [`Rig`] is what a scenario asks of an
//! issuer that the twin's traits do not give: its birth, its binding's capture, its line, and
//! the field edits a scenario makes to a row.
use super::super::{
    CharterId, FiledRevisionId, Holder, LicenceId, LineId, RevisionIdentity, TermGeneration,
    TermWindow, WorldIcSeconds,
    paid::{PaidTermRecord, PaidTerms, PersonalPaidLicence},
    records::{PublicationError, Record, RevisionPolicy},
};
use super::records::{RenewalAttemptV1, RenewalHeadV1, RenewalStateV1};
use super::{Binding, Closure, Operation, Rules, TermEconomics, chain};
use mush_core::{
    ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId,
    mechanics::{
        DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
    },
};
use mush_economy::{
    InstrumentId,
    renewal_twin::{
        Issuer as Economy, RenewalFundingReceiptV1, RenewalTerminalReceiptV1, UnfundedCloseReason,
    },
};
use mush_testkit::fixtures::{digest, stamp, token};
use serde_json::json;
use std::collections::BTreeMap;

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub(crate) mod capacity;
pub(crate) mod holder;

/// What a scenario asks of an issuer beyond the twin's traits.
pub(crate) trait Rig: Rules {
    /// The terms a structural birth is accepted under: an open-resale personal licence, or
    /// a sealed House one.
    fn birth_terms() -> TestResult<PaidTerms>;
    /// The issuer's grant over a renewal template of these terms and this policy.
    fn grant(terms: PaidTerms, policy: RevisionPolicy) -> TestResult<Self::Grant>;
    /// The same grant with its template's instrument replaced (the capacity scenario).
    fn widen(grant: &Self::Grant, instrument: InstrumentId) -> TestResult<Self::Grant>;
    /// The issuer's economics row.
    fn economics_row(
        licence: LicenceId,
        generation: TermGeneration,
        operation: CommitToken,
        product: DefinitionDigest,
        grant: Self::Grant,
    ) -> Result<Self::Economics, PublicationError>;
    /// The economics row's product digest and grant.
    fn product(economics: &Self::Economics) -> DefinitionDigest;
    fn grant_of(economics: &Self::Economics) -> &Self::Grant;
    /// The binding's capture (the House's is held to the House of the structural birth).
    fn capture(
        licence: &PersonalPaidLicence,
        term: &PaidTermRecord,
        economics: &Self::Economics,
    ) -> Result<Self::Binding, PublicationError>;
    /// The line of the structural birth, and another one where the issuer has one.
    fn birth_line() -> TestResult<Self::Line>;
    fn other_line() -> TestResult<Option<Self::Line>>;
    /// The field edits a scenario makes to a row.
    fn with_holder(row: &Self::Operation, holder: Holder) -> Self::Operation;
    fn with_attempt(row: &Self::Operation, attempt: RenewalAttemptV1) -> Self::Operation;
    fn with_offer_digest(row: &Self::Operation, digest: DefinitionDigest) -> Self::Operation;
    /// A transferred holder's binding: format 2, the holder and its basis.
    fn with_holder_basis(
        row: &Self::Operation,
        holder: Holder,
        basis: mush_economy::managed_paid_resale::RenewalHolderBasisV1,
    ) -> Self::Operation;
    /// The issuer's operation record decoded from its stored bytes.
    fn decode(bytes: &[u8]) -> Result<Self::Operation, PublicationError>;
    /// Economy's offer for a row's binding (the CT.0 walk's), and the same offer with the
    /// issuer's own side changed: the royalty chain's digest, or the House's scope.
    fn offer(row: &Self::Operation) -> TestResult<super::history::Offer<Self>>;
    fn off_side(offer: super::history::Offer<Self>) -> super::history::Offer<Self>;
}

pub(crate) fn definition(kind: &str, id: &str) -> TestResult<DefinitionRef> {
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
pub(crate) fn identity() -> TestResult<RevisionIdentity> {
    Ok(serde_json::from_value(
        json!({"format":1,"revision":{"key":{"character":1,"mind":1,"number":1},
        "digest":digest(1)},"source":digest(2)}),
    )?)
}
/// 17g C2g: `pub(crate)` with the helpers below, for Roll's command-read
/// tests (the named buyer's resale step on a granted licence).
pub(crate) struct Birth<I: Rig> {
    pub(crate) licence: PersonalPaidLicence,
    pub(crate) term: PaidTermRecord,
    pub(crate) economics: I::Economics,
}
pub(crate) fn birth<I: Rig>() -> TestResult<Birth<I>> {
    birth_with::<I>(I::birth_terms()?)
}
/// The structural birth over the given accepted terms (17g C2b: the marked
/// test builds them through the entitlement writer's own `personal_terms`).
pub(crate) fn birth_with<I: Rig>(terms: PaidTerms) -> TestResult<Birth<I>> {
    let id = LicenceId::new(7)?;
    let one = TermGeneration::new(1)?;
    let policy = RevisionPolicy::Pin {
        filed: FiledRevisionId::new(3)?,
        revision: identity()?,
    };
    let grant = I::grant(terms, policy)?;
    Ok(Birth {
        licence: PersonalPaidLicence::original(
            id,
            CharterId::new(9)?,
            LineId::new(2)?,
            ObjId(5).into(),
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
        economics: I::economics_row(id, one, token(1), digest(9), grant)?,
    })
}
pub(crate) fn draft<I: Rig>(
    b: &Birth<I>,
    n: u8,
    at: i64,
    attempt: RenewalAttemptV1,
) -> TestResult<I::Operation> {
    let row = I::Operation::drafted(
        token(n),
        definition("action", <I as Economy>::DRAFT_ACTION)?,
        attempt,
        I::capture(&b.licence, &b.term, &b.economics)?,
        digest(n),
        serde_json::from_value(json!(n))?,
        [n; 32],
        stamp(at),
        at,
    );
    row.validate()?;
    Ok(row)
}
pub(crate) fn funded<I: Rig>(row: &I::Operation) -> TestResult<I::Operation> {
    let n = u8::try_from(row.created_at_utc())?;
    let receipt: RenewalFundingReceiptV1<I> = serde_json::from_value(
        json!({"format":1,"commit":token(n+1),
        "action":definition("action",<I as Economy>::FUND_ACTION)?,"contract":row.contract(),"hold":u64::from(n),
        "transaction":u64::from(n),"journal_seq":u64::from(n),"consent_hash":row.consent_hash(),
        "at_ic":stamp(i64::from(n)+1),"posted_at_utc":i64::from(n)+1}),
    )?;
    let next = row.with_state(RenewalStateV1::Funded { funding: receipt });
    next.follows(row)?;
    Ok(next)
}
pub(crate) fn terminal<I: Rig>(row: &I::Operation, grant: bool) -> TestResult<I::Operation> {
    let funding = row.state().funding().ok_or("funding")?.clone();
    let n = u8::try_from(row.created_at_utc())?;
    let at = stamp(i64::from(n) + 2);
    let receipt: RenewalTerminalReceiptV1<I> = serde_json::from_value(
        json!({"format":1,"commit":token(n+2),
        "action":definition("action",if grant{<I as Economy>::GRANT_ACTION}else{<I as Economy>::REFUND_ACTION})?,
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
    let next = row.with_state(state);
    next.follows(row)?;
    Ok(next)
}
pub(crate) fn closed<I: Rig>(row: &I::Operation) -> TestResult<I::Operation> {
    let n = u8::try_from(row.created_at_utc())?;
    let next = row.with_state(RenewalStateV1::UnfundedClosed {
        closure: I::Closure::new(
            token(n + 2),
            definition("action", <I as Economy>::CLOSE_ACTION)?,
            stamp(i64::from(n) + 2),
            i64::from(n) + 2,
            UnfundedCloseReason::Declined,
        ),
    });
    next.follows(row)?;
    Ok(next)
}
pub(crate) fn granted_birth<I: Rig>(row: &I::Operation) -> TestResult<Birth<I>> {
    let (licence, term, economics) = row.granted_rows()?;
    Ok(Birth {
        licence,
        term,
        economics,
    })
}

/// Every scenario of this module, run as a `#[test]` under its name in the family's test
/// module (`$crate::commerce::<family>::tests`), with the family's `holder` and
/// `capacity_tests` modules as before. A body left out of the list is dead code, which
/// `-D warnings` refuses.
macro_rules! instantiate {
    ($issuer:ty) => {
        $crate::commerce::renewal_twin::tests::instantiate!(@tests $issuer, () {
            strict_rows_reject_missing_null_duplicate_and_nested_unknown_fields,
            closed_attempt_can_precede_exact_retry_grant_without_eternal_next_absence,
            retry_chain_rejects_branch_orphan_wrong_holder_and_automatic_repeat,
            current_generation_advances_without_rewriting_birth_or_historical_acquisition,
            pending_funding_preserves_old_current_rows_and_blocks_new_attempt,
            exact_transition_and_issued_clock_refuse_crossed_receipts,
            exhausted_generation_refuses_before_any_new_offer_or_money,
            an_offer_off_the_issuers_side_of_the_binding_is_refused,
            the_issuer_seal_admits_exactly_two,
        });
        mod holder {
            $crate::commerce::renewal_twin::tests::instantiate!(@tests $issuer, (holder::) {
                holder_binding_preserves_old_bytes_and_requires_exact_new_attempt_evidence,
                transferred_buyer_automatic_then_own_retry_preserves_holder_after_grant,
                repeated_transfer_gets_new_automatic_but_cannot_replace_pending_renewal,
            });
        }
        mod capacity_tests {
            $crate::commerce::renewal_twin::tests::instantiate!(@tests $issuer, (capacity::) {
                admitted_widest_economics_keeps_all_actual_roll_terminal_envelopes_bounded,
            });
        }
    };
    (@tests $issuer:ty, $prefix:tt { $($name:ident),* $(,)? }) => {
        $(
            $crate::commerce::renewal_twin::tests::instantiate!(@one $issuer, $prefix, $name);
        )*
    };
    (@one $issuer:ty, ($($prefix:tt)*), $name:ident) => {
        #[test]
        fn $name() -> $crate::commerce::renewal_twin::tests::TestResult {
            $crate::commerce::renewal_twin::tests::$($prefix)*$name::<$issuer>()
        }
    };
}
pub(crate) use instantiate;

pub(crate) fn strict_rows_reject_missing_null_duplicate_and_nested_unknown_fields<I: Rig>()
-> TestResult {
    let row = draft::<I>(&birth::<I>()?, 10, 20, RenewalAttemptV1::Automatic {})?;
    let raw = serde_json::to_vec(&row)?;
    check_eq!(I::decode(&raw)?, row);
    let repeated = String::from_utf8(raw)?.replacen("{", "{\"format\":1,", 1);
    check!(I::decode(repeated.as_bytes()).is_err());
    let mut value = serde_json::to_value(&row)?;
    value
        .get_mut("state")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("state")?
        .insert("allowed".into(), json!(true));
    check!(serde_json::from_value::<I::Operation>(value).is_err());
    let head = RenewalHeadV1::<I>::for_operation(&closed::<I>(&row)?)?;
    let mut value = serde_json::to_value(&head)?;
    check_eq!(value.get("pending"), Some(&serde_json::Value::Null));
    value.as_object_mut().ok_or("head")?.remove("pending");
    check!(serde_json::from_value::<RenewalHeadV1<I>>(value).is_err());
    check!(
        serde_json::from_str::<RenewalAttemptV1>(
            r#"{"kind":"automatic","previous_operation":null}"#
        )
        .is_err()
    );
    check!(I::decode(&vec![b' '; super::records::MAX_OPERATION_BYTES + 1]).is_err());
    Ok(())
}

pub(crate) fn closed_attempt_can_precede_exact_retry_grant_without_eternal_next_absence<I: Rig>()
-> TestResult {
    let b = birth::<I>()?;
    let closed = closed::<I>(&draft::<I>(&b, 90, 20, RenewalAttemptV1::Automatic {})?)?;
    let retry = draft::<I>(
        &b,
        10,
        40,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: closed.operation(),
        },
    )?;
    chain::check_draft::<I>(
        &b.licence,
        &retry,
        Some(&RenewalHeadV1::<I>::for_operation(&closed)?),
        Some(&closed),
    )?;
    let granted = terminal::<I>(&funded::<I>(&retry)?, true)?;
    let current = granted_birth::<I>(&granted)?;
    let terms = BTreeMap::from([
        (b.term.generation(), &b.term),
        (current.term.generation(), &current.term),
    ]);
    let economics = BTreeMap::from([
        (b.economics.generation(), &b.economics),
        (current.economics.generation(), &current.economics),
    ]);
    // Lexical token order is deliberately the opposite of causal order.
    chain::verify::<I>(
        &current.licence,
        I::birth_line()?,
        Some(&RenewalHeadV1::<I>::for_operation(&granted)?),
        &[&granted, &closed],
        &terms,
        &economics,
    )?;
    // The accepted House issuer is immutable across holder and term history.
    if let Some(other) = I::other_line()? {
        check!(
            chain::verify::<I>(
                &current.licence,
                other,
                Some(&RenewalHeadV1::<I>::for_operation(&granted)?),
                &[&granted, &closed],
                &terms,
                &economics,
            )
            .is_err()
        );
    }
    check!(
        chain::verify::<I>(
            &current.licence,
            I::birth_line()?,
            Some(&RenewalHeadV1::<I>::for_operation(&granted)?),
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
        .insert("operation".into(), json!(closed.operation()));
    wrong = serde_json::from_value(wire)?;
    let wrong_terms =
        BTreeMap::from([(b.term.generation(), &b.term), (wrong.generation(), &wrong)]);
    check!(
        chain::verify::<I>(
            &current.licence,
            I::birth_line()?,
            Some(&RenewalHeadV1::<I>::for_operation(&granted)?),
            &[&closed, &granted],
            &wrong_terms,
            &economics
        )
        .is_err()
    );
    Ok(())
}

pub(crate) fn retry_chain_rejects_branch_orphan_wrong_holder_and_automatic_repeat<I: Rig>()
-> TestResult {
    let b = birth::<I>()?;
    let refund = terminal::<I>(
        &funded::<I>(&draft::<I>(&b, 80, 20, RenewalAttemptV1::Automatic {})?)?,
        false,
    )?;
    let head = RenewalHeadV1::<I>::for_operation(&refund)?;
    let retry = draft::<I>(
        &b,
        10,
        40,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: refund.operation(),
        },
    )?;
    let branch = draft::<I>(&b, 11, 50, retry.attempt())?;
    let terms = BTreeMap::from([(b.term.generation(), &b.term)]);
    let economics = BTreeMap::from([(b.economics.generation(), &b.economics)]);
    chain::verify::<I>(
        &b.licence,
        I::birth_line()?,
        Some(&RenewalHeadV1::<I>::for_operation(&retry)?),
        &[&retry, &refund],
        &terms,
        &economics,
    )?;
    check!(
        chain::verify::<I>(
            &b.licence,
            I::birth_line()?,
            Some(&head),
            &[&refund, &retry, &branch],
            &terms,
            &economics
        )
        .is_err()
    );
    check!(
        chain::check_draft::<I>(
            &b.licence,
            &draft::<I>(&b, 12, 40, RenewalAttemptV1::Automatic {})?,
            Some(&head),
            Some(&refund)
        )
        .is_err()
    );
    let wrong = I::with_holder(&retry, ObjId(6).into());
    check!(chain::check_draft::<I>(&b.licence, &wrong, Some(&head), Some(&refund)).is_err());
    let orphan = I::with_attempt(
        &retry,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: token(99),
        },
    );
    check!(
        chain::verify::<I>(
            &b.licence,
            I::birth_line()?,
            Some(&RenewalHeadV1::<I>::for_operation(&orphan)?),
            &[&refund, &orphan],
            &terms,
            &economics
        )
        .is_err()
    );
    Ok(())
}

pub(crate) fn current_generation_advances_without_rewriting_birth_or_historical_acquisition<
    I: Rig,
>() -> TestResult {
    let b = birth::<I>()?;
    let first = terminal::<I>(
        &funded::<I>(&draft::<I>(&b, 80, 20, RenewalAttemptV1::Automatic {})?)?,
        true,
    )?;
    let second_birth = granted_birth::<I>(&first)?;
    let second = terminal::<I>(
        &funded::<I>(&draft::<I>(
            &second_birth,
            10,
            40,
            RenewalAttemptV1::Automatic {},
        )?)?,
        true,
    )?;
    let current = granted_birth::<I>(&second)?;
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
    chain::verify::<I>(
        &current.licence,
        I::birth_line()?,
        Some(&RenewalHeadV1::<I>::for_operation(&second)?),
        &[&second, &first],
        &terms,
        &economics,
    )?;
    check_eq!(current.licence.operation(), b.licence.operation());
    check_eq!(current.term.operation(), second.operation());
    first
        .binding()
        .check_predecessor(&current.licence, &b.term, &b.economics)?;
    second.binding().check_predecessor(
        &current.licence,
        &second_birth.term,
        &second_birth.economics,
    )?;
    check!(b.term.matches_current(&current.licence).is_err());
    check!(
        chain::verify::<I>(
            &current.licence,
            I::birth_line()?,
            None,
            &[&first, &second],
            &terms,
            &economics
        )
        .is_err()
    );
    Ok(())
}

pub(crate) fn pending_funding_preserves_old_current_rows_and_blocks_new_attempt<I: Rig>()
-> TestResult {
    let b = birth::<I>()?;
    let old = (
        serde_json::to_vec(&b.licence)?,
        serde_json::to_vec(&b.term)?,
        serde_json::to_vec(&b.economics)?,
    );
    let draft = draft::<I>(&b, 80, 20, RenewalAttemptV1::Automatic {})?;
    let funded = funded::<I>(&draft)?;
    let head = RenewalHeadV1::<I>::for_operation(&funded)?;
    check_eq!(head, RenewalHeadV1::<I>::for_operation(&draft)?);
    chain::verify::<I>(
        &b.licence,
        I::birth_line()?,
        Some(&head),
        &[&funded],
        &BTreeMap::from([(b.term.generation(), &b.term)]),
        &BTreeMap::from([(b.economics.generation(), &b.economics)]),
    )?;
    check!(funded.granted_rows().is_err());
    check!(chain::check_draft::<I>(&b.licence, &draft, Some(&head), Some(&funded)).is_err());
    check!(
        old == (
            serde_json::to_vec(&b.licence)?,
            serde_json::to_vec(&b.term)?,
            serde_json::to_vec(&b.economics)?
        ),
        "pending renewal rewrote issued rows"
    );
    Ok(())
}

pub(crate) fn exact_transition_and_issued_clock_refuse_crossed_receipts<I: Rig>() -> TestResult {
    let b = birth::<I>()?;
    let draft = draft::<I>(&b, 80, 20, RenewalAttemptV1::Automatic {})?;
    let funded = funded::<I>(&draft)?;
    let granted = terminal::<I>(&funded, true)?;
    check!(granted.follows(&draft).is_err());
    check!(granted.follows(&granted).is_err());
    let crossed = I::with_offer_digest(&granted, digest(99));
    check!(crossed.follows(&funded).is_err());
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
    check!(serde_json::from_value::<I::Operation>(wire).is_err());
    let refund = terminal_refund_clock::<I>(&funded, 2)?;
    refund.follows(&funded)?;
    check!(refund.granted_rows().is_err());
    Ok(())
}
fn terminal_refund_clock<I: Rig>(
    funded: &I::Operation,
    generation: u64,
) -> TestResult<I::Operation> {
    let refund = terminal::<I>(funded, false)?;
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

pub(crate) fn exhausted_generation_refuses_before_any_new_offer_or_money<I: Rig>() -> TestResult {
    let b = birth::<I>()?;
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
    let economics = I::economics_row(
        licence.id(),
        max,
        token(9),
        I::product(&b.economics),
        I::grant_of(&b.economics).clone(),
    )?;
    check!(I::capture(&licence, &term, &economics).is_err());
    Ok(())
}

/// Doc 46 CT.5 (mutation H-B): the issuer's side of an offer is held to the binding, the
/// royalty chain's digest for the personal issuer, the House's organisation and scope for
/// the House. Only that one field differs between the two offers here, and the row carries
/// each offer's own digest, so the refusal is the issuer's check and no other.
pub(crate) fn an_offer_off_the_issuers_side_of_the_binding_is_refused<I: Rig>() -> TestResult {
    use mush_economy::renewal_twin::Offer as _;
    let row = draft::<I>(&birth::<I>()?, 10, 20, RenewalAttemptV1::Automatic {})?;
    let offer = I::offer(&row)?;
    let row = I::with_offer_digest(&row, offer.digest()?);
    super::history::check_offer::<I>(&row, &offer)?;
    let off = I::off_side(offer);
    let row = I::with_offer_digest(&row, off.digest()?);
    check!(matches!(
        super::history::check_offer::<I>(&row, &off),
        Err(PublicationError::Mismatch)
    ));
    Ok(())
}

/// The census for a later edit that widens the seal instead of going round it: exactly two
/// `impl Sealed for` in the twin's `issuer.rs`; one `impl Issuer for` and one
/// `impl Rules for` in each family's `mod.rs`; one `mod sealed {` of the twin's, and no
/// `dyn` beside either trait's name. A census of the source, not a proof. Run for both
/// issuers, which read the same files.
pub(crate) fn the_issuer_seal_admits_exactly_two<I: Rig>() -> TestResult {
    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> TestResult {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                sources(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files)?;
    files.sort();
    let dynamic = [["dyn ", "Issuer"].concat(), ["dyn ", "Rules"].concat()];
    let (mut sealed, mut issuers, mut rules) = (Vec::new(), Vec::new(), Vec::new());
    for path in &files {
        let name = path
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        for line in std::fs::read_to_string(path)?.lines() {
            let line = line.trim();
            if line.starts_with(
                ["impl Sealed for ", "mush_economy::renewal_twin::"]
                    .concat()
                    .as_str(),
            ) {
                sealed.push(name.clone());
            }
            if line.starts_with("impl Issuer for ") {
                issuers.push(name.clone());
            }
            if line.starts_with("impl Rules for ") {
                rules.push(name.clone());
            }
            for d in &dynamic {
                check!(!line.contains(d.as_str()), "{name}: {line}");
            }
        }
    }
    check_eq!(
        sealed,
        [
            "commerce/renewal_twin/issuer.rs",
            "commerce/renewal_twin/issuer.rs"
        ]
    );
    check_eq!(
        issuers,
        ["commerce/house_renewal/mod.rs", "commerce/renewal/mod.rs"]
    );
    check_eq!(
        rules,
        ["commerce/house_renewal/mod.rs", "commerce/renewal/mod.rs"]
    );
    let twin = std::fs::read_to_string(root.join("commerce/renewal_twin/issuer.rs"))?;
    check_eq!(
        twin.lines()
            .filter(|l| l.trim() == ["mod ", "sealed {"].concat())
            .count(),
        1
    );
    check_eq!(
        std::any::type_name::<I>().matches("renewal_twin").count(),
        1
    );
    Ok(())
}

/// The readers' service scenarios, one text run in both families' `reader::service_tests`
/// (the reader is each family's own; these read only what both readers have alike).
macro_rules! reader_service_tests {
    () => {
        type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
        fn manifest() -> TestResult<DefinitionRef> {
            Ok(DefinitionRegistryBuilder::new().register(
                OwnerId::new("tacsim")?,
                DefinitionDescriptor::new(
                    DefinitionKind::new("registry_manifest")?,
                    DefinitionId::new("combat")?,
                    1,
                    serde_json::json!({"fixture":1}),
                )?,
            )?)
        }
        #[test]
        fn service_reader_keeps_old_descriptor_and_own_wire_but_versions_inventory() -> TestResult {
            let manifest = manifest()?;
            check_eq!(
                configured_descriptor(&manifest, false)?,
                descriptor(&manifest)?
            );
            let old = DefinitionRegistryBuilder::new()
                .register(OwnerId::new("roll")?, descriptor(&manifest)?)?;
            let service = DefinitionRegistryBuilder::new().register(
                OwnerId::new("roll")?,
                configured_descriptor(&manifest, true)?,
            )?;
            check_eq!(old.key(), service.key());
            check_eq!(old.version(), 1);
            check_eq!(service.version(), 2);
            check_ne!(old.digest(), service.digest());
            let licence = LicenceId::new(7)?;
            check_eq!(
                serde_json::to_value(RenewalRequest::own(licence))?,
                serde_json::json!({"licence":licence})
            );
            check_eq!(
                serde_json::to_value(RenewalRequest::inventory(None))?,
                serde_json::json!({"inventory_after":null})
            );
            check_eq!(
                serde_json::to_value(RenewalRequest::status(licence))?,
                serde_json::json!({"status_licence":licence})
            );
            Ok(())
        }
        /// Doc 44 MW.2 (Q-44-14): the lasting refusals, and their order. Nothing
        /// passing is one, so the service keeps trying everything else.
        #[test]
        fn a_recalled_or_withdrawn_revision_and_an_owned_charter_stop_a_renewal() -> TestResult {
            check_eq!(RenewalStop::of(false, false, false), None);
            check_eq!(
                RenewalStop::of(true, false, false),
                Some(RenewalStop::Recalled)
            );
            check_eq!(
                RenewalStop::of(false, true, false),
                Some(RenewalStop::Withdrawn)
            );
            check_eq!(
                RenewalStop::of(false, false, true),
                Some(RenewalStop::OwnCharter)
            );
            // The Registry's act is told before the author's, either before the title.
            check_eq!(
                RenewalStop::of(true, true, true),
                Some(RenewalStop::Recalled)
            );
            check_eq!(
                RenewalStop::of(false, true, true),
                Some(RenewalStop::Withdrawn)
            );
            check_eq!(
                serde_json::to_value(RenewalStop::OwnCharter)?,
                serde_json::json!("own_charter")
            );
            Ok(())
        }
        /// An entry that is not stopped serialises exactly as before the field
        /// (the service descriptor is unchanged); a stopped one carries its cause.
        #[test]
        fn the_stop_is_absent_from_the_wire_unless_set() -> TestResult {
            let entry = |stop| RenewalInventoryEntry {
                licence: LicenceId::new(7).unwrap(),
                holder: Holder::Character(mush_core::ObjId(2)),
                generation: super::super::super::TermGeneration::new(1).unwrap(),
                term_end: WorldIcSeconds(10),
                last_operation: None,
                attempted: false,
                holder_generation: None,
                stop,
            };
            let plain = serde_json::to_value(entry(None))?;
            check!(plain.get("stop").is_none());
            check_eq!(
                serde_json::to_value(entry(Some(RenewalStop::Recalled)))?
                    .get("stop")
                    .cloned(),
                Some(serde_json::json!("recalled"))
            );
            check_eq!(
                entry(Some(RenewalStop::Withdrawn)).stop(),
                Some(RenewalStop::Withdrawn)
            );
            Ok(())
        }
    };
}
pub(crate) use reader_service_tests;
