//! Structural attempt/holder joins only. Native settlement and current owner
//! authority are exercised separately by the actual owner/genuine fixtures.
use super::*;
use mush_economy::managed_paid_resale::{PaidHolderRefV1, RenewalHolderBasisV1};

fn basis(generation: u64) -> RenewalHolderBasisV1 {
    RenewalHolderBasisV1 {
        acceptance: token(210 + generation as u8),
        ownership: PaidHolderRefV1 {
            generation,
            operation: token(220 + generation as u8),
            digest: digest(230 + generation as u8),
        },
    }
}
fn holder_draft(
    b: &Birth,
    n: u8,
    at: i64,
    generation: u64,
    previous: Option<CommitToken>,
) -> TestResult<RenewalOperationV1> {
    let mut row = draft(b, n, at, RenewalAttemptV1::Automatic {})?;
    row.binding.format = 2;
    row.binding.holder = mush_core::ObjId(40 + u32::try_from(generation)?);
    row.binding.holder_basis = Some(basis(generation));
    row.attempt = match previous {
        None => RenewalAttemptV1::HolderAutomatic {
            basis: basis(generation),
        },
        Some(previous_operation) => RenewalAttemptV1::HolderRetry {
            basis: basis(generation),
            previous_operation,
        },
    };
    row.validate()?;
    Ok(row)
}

#[test]
fn holder_binding_preserves_old_bytes_and_requires_exact_new_attempt_evidence() -> TestResult {
    let b = birth()?;
    let old = draft(&b, 90, 20, RenewalAttemptV1::Automatic {})?;
    let old_wire = serde_json::to_value(&old.binding)?;
    assert!(old_wire.get("holder_basis").is_none());
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<RenewalBindingV1>(
            old_wire.clone()
        )?)?,
        old_wire
    );
    let row = holder_draft(&b, 10, 40, 2, None)?;
    let wire = serde_json::to_value(&row.binding)?;
    assert_eq!(
        serde_json::from_value::<RenewalBindingV1>(wire.clone())?,
        *row.binding
    );
    for bad in [
        serde_json::Value::Null,
        serde_json::json!({"acceptance":token(212),"ownership":{"generation":1,"operation":token(222),"digest":digest(232)}}),
    ] {
        let mut value = wire.clone();
        value["holder_basis"] = bad;
        assert!(serde_json::from_value::<RenewalBindingV1>(value).is_err());
    }
    let mut missing = wire.clone();
    missing
        .as_object_mut()
        .ok_or("binding")?
        .remove("holder_basis");
    assert!(serde_json::from_value::<RenewalBindingV1>(missing).is_err());
    let mut old_null = old_wire.clone();
    old_null["holder_basis"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<RenewalBindingV1>(old_null).is_err());
    let mut old_extra = old_wire;
    old_extra["holder_basis"] = serde_json::to_value(basis(2))?;
    assert!(serde_json::from_value::<RenewalBindingV1>(old_extra).is_err());
    let mut crossed = row;
    crossed.attempt = RenewalAttemptV1::HolderAutomatic { basis: basis(3) };
    assert!(crossed.validate().is_err());
    Ok(())
}

#[inline(never)]
fn granted_holder_cases() -> TestResult<Box<GrantedHolderCases>> {
    let b = birth()?;
    let old = closed(&draft(&b, 90, 20, RenewalAttemptV1::Automatic {})?)?;
    let automatic = holder_draft(&b, 70, 40, 2, None)?;
    let pointer = automatic.binding.accepted_pointer(b.licence.term())?;
    chain::check_draft(
        &pointer,
        &automatic,
        Some(&RenewalHeadV1::for_operation(&old)?),
        Some(&old),
    )?;
    let buyer_closed = closed(&automatic)?;
    let retry = holder_draft(&b, 10, 60, 2, Some(buyer_closed.operation))?;
    chain::check_draft(
        &pointer,
        &retry,
        Some(&RenewalHeadV1::for_operation(&buyer_closed)?),
        Some(&buyer_closed),
    )?;
    let grant = terminal(&funded(&retry)?, true)?;
    let next = granted_birth(&grant)?;
    assert_eq!(next.licence.ownership(), pointer.ownership());
    assert_eq!(next.licence.holder(), pointer.holder());
    assert_eq!(next.licence.operation(), b.licence.operation());
    let terms = BTreeMap::from([
        (b.term.generation(), &b.term),
        (next.term.generation(), &next.term),
    ]);
    let economics = BTreeMap::from([
        (b.economics.generation(), &b.economics),
        (next.economics.generation(), &next.economics),
    ]);
    chain::verify(
        &next.licence,
        Some(&RenewalHeadV1::for_operation(&grant)?),
        &[&grant, &old, &buyer_closed],
        &terms,
        &economics,
    )?;
    Ok(Box::new(GrantedHolderCases {
        b,
        old,
        automatic,
        pointer,
        buyer_closed,
    }))
}

struct GrantedHolderCases {
    b: Birth,
    old: RenewalOperationV1,
    automatic: RenewalOperationV1,
    pointer: crate::commerce::paid::PersonalPaidLicence,
    buyer_closed: RenewalOperationV1,
}

#[test]
fn transferred_buyer_automatic_then_own_retry_preserves_holder_after_grant() -> TestResult {
    let GrantedHolderCases {
        b,
        old,
        automatic,
        pointer,
        buyer_closed,
    } = *granted_holder_cases()?;
    let seller_retry = draft(
        &b,
        11,
        40,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: old.operation,
        },
    )?;
    assert!(
        chain::check_draft(
            &pointer,
            &seller_retry,
            Some(&RenewalHeadV1::for_operation(&old)?),
            Some(&old)
        )
        .is_err()
    );
    assert!(
        chain::check_draft(
            &pointer,
            &automatic,
            Some(&RenewalHeadV1::for_operation(&buyer_closed)?),
            Some(&buyer_closed)
        )
        .is_err()
    );
    let bad_retry = holder_draft(&b, 12, 60, 2, Some(old.operation))?;
    assert!(
        chain::check_draft(
            &pointer,
            &bad_retry,
            Some(&RenewalHeadV1::for_operation(&buyer_closed)?),
            Some(&buyer_closed)
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn repeated_transfer_gets_new_automatic_but_cannot_replace_pending_renewal() -> TestResult {
    let b = birth()?;
    let first = holder_draft(&b, 90, 20, 2, None)?;
    let second = holder_draft(&b, 10, 40, 3, None)?;
    let pointer = second.binding.accepted_pointer(b.licence.term())?;
    assert!(
        chain::check_draft(
            &pointer,
            &second,
            Some(&RenewalHeadV1::for_operation(&first)?),
            Some(&first)
        )
        .is_err()
    );
    let closed = closed(&first)?;
    chain::check_draft(
        &pointer,
        &second,
        Some(&RenewalHeadV1::for_operation(&closed)?),
        Some(&closed),
    )?;
    let terms = BTreeMap::from([(b.term.generation(), &b.term)]);
    let economics = BTreeMap::from([(b.economics.generation(), &b.economics)]);
    chain::verify(
        &pointer,
        Some(&RenewalHeadV1::for_operation(&second)?),
        &[&second, &closed],
        &terms,
        &economics,
    )?;
    assert!(
        chain::verify(
            &pointer,
            Some(&RenewalHeadV1::for_operation(&second)?),
            &[&second, &first],
            &terms,
            &economics
        )
        .is_err()
    );
    Ok(())
}
