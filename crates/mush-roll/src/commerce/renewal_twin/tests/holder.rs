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
fn holder_draft<I: Rig>(
    b: &Birth<I>,
    n: u8,
    at: i64,
    generation: u64,
    previous: Option<CommitToken>,
) -> TestResult<I::Operation> {
    let row = draft::<I>(b, n, at, RenewalAttemptV1::Automatic {})?;
    let row = I::with_holder_basis(
        &row,
        mush_core::ObjId(40 + u32::try_from(generation)?).into(),
        basis(generation),
    );
    let row = I::with_attempt(
        &row,
        match previous {
            None => RenewalAttemptV1::HolderAutomatic {
                basis: basis(generation),
            },
            Some(previous_operation) => RenewalAttemptV1::HolderRetry {
                basis: basis(generation),
                previous_operation,
            },
        },
    );
    row.validate()?;
    Ok(row)
}

pub(crate) fn holder_binding_preserves_old_bytes_and_requires_exact_new_attempt_evidence<I: Rig>()
-> TestResult {
    let b = birth::<I>()?;
    let old = draft::<I>(&b, 90, 20, RenewalAttemptV1::Automatic {})?;
    let old_wire = serde_json::to_value(old.binding())?;
    check!(old_wire.get("holder_basis").is_none());
    check_eq!(
        serde_json::to_value(serde_json::from_value::<I::Binding>(old_wire.clone())?)?,
        old_wire
    );
    let row = holder_draft::<I>(&b, 10, 40, 2, None)?;
    let wire = serde_json::to_value(row.binding())?;
    check_eq!(
        &serde_json::from_value::<I::Binding>(wire.clone())?,
        row.binding()
    );
    for bad in [
        serde_json::Value::Null,
        serde_json::json!({"acceptance":token(212),"ownership":{"generation":1,"operation":token(222),"digest":digest(232)}}),
    ] {
        let mut value = wire.clone();
        value["holder_basis"] = bad;
        check!(serde_json::from_value::<I::Binding>(value).is_err());
    }
    let mut missing = wire.clone();
    missing
        .as_object_mut()
        .ok_or("binding")?
        .remove("holder_basis");
    check!(serde_json::from_value::<I::Binding>(missing).is_err());
    let mut old_null = old_wire.clone();
    old_null["holder_basis"] = serde_json::Value::Null;
    check!(serde_json::from_value::<I::Binding>(old_null).is_err());
    let mut old_extra = old_wire;
    old_extra["holder_basis"] = serde_json::to_value(basis(2))?;
    check!(serde_json::from_value::<I::Binding>(old_extra).is_err());
    let crossed = I::with_attempt(&row, RenewalAttemptV1::HolderAutomatic { basis: basis(3) });
    check!(crossed.validate().is_err());
    Ok(())
}

#[inline(never)]
fn granted_holder_cases<I: Rig>() -> TestResult<Box<GrantedHolderCases<I>>> {
    let b = Box::new(birth::<I>()?);
    let old = closed::<I>(&draft::<I>(&b, 90, 20, RenewalAttemptV1::Automatic {})?)?;
    let automatic = holder_draft::<I>(&b, 70, 40, 2, None)?;
    let pointer = automatic.binding().accepted_pointer(b.licence.term())?;
    chain::check_draft::<I>(
        &pointer,
        &automatic,
        Some(&RenewalHeadV1::<I>::for_operation(&old)?),
        Some(&old),
    )?;
    let buyer_closed = closed::<I>(&automatic)?;
    let retry = holder_draft::<I>(&b, 10, 60, 2, Some(buyer_closed.operation()))?;
    chain::check_draft::<I>(
        &pointer,
        &retry,
        Some(&RenewalHeadV1::<I>::for_operation(&buyer_closed)?),
        Some(&buyer_closed),
    )?;
    let grant = terminal::<I>(&funded::<I>(&retry)?, true)?;
    let next = granted_birth::<I>(&grant)?;
    check_eq!(next.licence.ownership(), pointer.ownership());
    check_eq!(next.licence.holder(), pointer.holder());
    check_eq!(next.licence.operation(), b.licence.operation());
    let terms = BTreeMap::from([
        (b.term.generation(), &b.term),
        (next.term.generation(), &next.term),
    ]);
    let economics = BTreeMap::from([
        (b.economics.generation(), &b.economics),
        (next.economics.generation(), &next.economics),
    ]);
    chain::verify::<I>(
        &next.licence,
        I::birth_line()?,
        Some(&RenewalHeadV1::<I>::for_operation(&grant)?),
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

struct GrantedHolderCases<I: Rig> {
    b: Box<Birth<I>>,
    old: I::Operation,
    automatic: I::Operation,
    pointer: PersonalPaidLicence,
    buyer_closed: I::Operation,
}

pub(crate) fn transferred_buyer_automatic_then_own_retry_preserves_holder_after_grant<I: Rig>()
-> TestResult {
    let GrantedHolderCases {
        b,
        old,
        automatic,
        pointer,
        buyer_closed,
    } = *granted_holder_cases::<I>()?;
    let seller_retry = draft::<I>(
        &b,
        11,
        40,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: old.operation(),
        },
    )?;
    check!(
        chain::check_draft::<I>(
            &pointer,
            &seller_retry,
            Some(&RenewalHeadV1::<I>::for_operation(&old)?),
            Some(&old)
        )
        .is_err()
    );
    check!(
        chain::check_draft::<I>(
            &pointer,
            &automatic,
            Some(&RenewalHeadV1::<I>::for_operation(&buyer_closed)?),
            Some(&buyer_closed)
        )
        .is_err()
    );
    let bad_retry = holder_draft::<I>(&b, 12, 60, 2, Some(old.operation()))?;
    check!(
        chain::check_draft::<I>(
            &pointer,
            &bad_retry,
            Some(&RenewalHeadV1::<I>::for_operation(&buyer_closed)?),
            Some(&buyer_closed)
        )
        .is_err()
    );
    Ok(())
}

pub(crate) fn repeated_transfer_gets_new_automatic_but_cannot_replace_pending_renewal<I: Rig>()
-> TestResult {
    let b = birth::<I>()?;
    let first = holder_draft::<I>(&b, 90, 20, 2, None)?;
    let second = holder_draft::<I>(&b, 10, 40, 3, None)?;
    let pointer = second.binding().accepted_pointer(b.licence.term())?;
    check!(
        chain::check_draft::<I>(
            &pointer,
            &second,
            Some(&RenewalHeadV1::<I>::for_operation(&first)?),
            Some(&first)
        )
        .is_err()
    );
    let closed = closed::<I>(&first)?;
    chain::check_draft::<I>(
        &pointer,
        &second,
        Some(&RenewalHeadV1::<I>::for_operation(&closed)?),
        Some(&closed),
    )?;
    let terms = BTreeMap::from([(b.term.generation(), &b.term)]);
    let economics = BTreeMap::from([(b.economics.generation(), &b.economics)]);
    chain::verify::<I>(
        &pointer,
        I::birth_line()?,
        Some(&RenewalHeadV1::<I>::for_operation(&second)?),
        &[&second, &closed],
        &terms,
        &economics,
    )?;
    check!(
        chain::verify::<I>(
            &pointer,
            I::birth_line()?,
            Some(&RenewalHeadV1::<I>::for_operation(&second)?),
            &[&second, &first],
            &terms,
            &economics
        )
        .is_err()
    );
    Ok(())
}
