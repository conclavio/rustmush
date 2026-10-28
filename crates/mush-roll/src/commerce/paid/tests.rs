#![cfg(test)]

use mush_core::ObjId;
use serde_json::{Value, json};

use super::*;
use crate::commerce::{MAX_TERM_SECONDS, TermPhase, WorldIcSeconds};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn require(ok: bool, message: &'static str) -> TestResult {
    if ok { Ok(()) } else { Err(message.into()) }
}
fn object(
    value: &mut Value,
) -> Result<&mut serde_json::Map<String, Value>, Box<dyn std::error::Error>> {
    value
        .as_object_mut()
        .ok_or_else(|| "fixture is not an object".into())
}
fn nested<'a>(
    value: &'a mut Value,
    field: &str,
) -> Result<&'a mut Value, Box<dyn std::error::Error>> {
    value
        .get_mut(field)
        .ok_or_else(|| "fixture has no field".into())
}
fn set(value: &mut Value, field: &str, item: Value) -> TestResult {
    object(value)?.insert(field.into(), item);
    Ok(())
}
fn licence_value() -> Value {
    json!({"format":1,"id":"4","charter":"20","line":"3","holder":7,
        "term":"1","operation":CommitToken::from_bytes([1;16])})
}
fn term_value() -> Value {
    json!({"format":1,"licence":"4","generation":"1",
        "window":{"format":1,"eligibility_start":100,"issued_at":100,"term_end":200,"grace_end":210},
        "terms":{"seats":1,"class":"II","form":{"kind":"sealed"},"transferable":true},
        "revision":{"kind":"follow","line":"3"},"operation":CommitToken::from_bytes([1;16])})
}
fn licence(value: &Value) -> Result<PersonalPaidLicence, Box<dyn std::error::Error>> {
    Ok(PersonalPaidLicence::decode(&serde_json::to_vec(value)?)?)
}
fn term(value: &Value) -> Result<PaidTermRecord, Box<dyn std::error::Error>> {
    Ok(PaidTermRecord::decode(&serde_json::to_vec(value)?)?)
}
fn refused_licence(value: &Value) -> TestResult {
    require(
        PersonalPaidLicence::decode(&serde_json::to_vec(value)?).is_err(),
        "invalid licence decoded",
    )
}
fn refused_term(value: &Value) -> TestResult {
    require(
        PaidTermRecord::decode(&serde_json::to_vec(value)?).is_err(),
        "invalid term decoded",
    )
}

#[test]
fn source_form_and_licence_transferability_are_independent_closed_terms() -> TestResult {
    for (name, expected) in [
        ("sealed", PaidForm::Sealed {}),
        ("open", PaidForm::Open {}),
        ("open_resale", PaidForm::OpenResale {}),
    ] {
        for transferable in [false, true] {
            let mut value = term_value();
            let terms = nested(&mut value, "terms")?;
            set(terms, "form", json!({"kind":name}))?;
            set(terms, "transferable", json!(transferable))?;
            let decoded = term(&value)?;
            require(decoded.terms().form() == expected, "form changed")?;
            require(
                decoded.terms().transferable() == transferable,
                "transfer fact changed",
            )?;
            require(
                decoded.terms().seats().get() == 1 && decoded.terms().class() == RatingClass::II,
                "paid consent changed",
            )?;
            require(
                serde_json::to_value(decoded)? == value,
                "term did not round trip",
            )?;
        }
    }
    let value = licence_value();
    require(
        serde_json::to_value(licence(&value)?)? == value,
        "licence did not round trip",
    )
}

#[test]
fn unknown_fields_refuse_at_every_nested_paid_boundary() -> TestResult {
    for path in [
        vec![],
        vec!["window"],
        vec!["terms"],
        vec!["terms", "form"],
        vec!["revision"],
    ] {
        let mut value = term_value();
        let mut target = &mut value;
        for field in path {
            target = nested(target, field)?;
        }
        set(target, "granted", json!(true))?;
        refused_term(&value)?;
    }
    for name in ["sealed", "open", "open_resale"] {
        let mut value = term_value();
        set(
            nested(&mut value, "terms")?,
            "form",
            json!({"kind":name,"holder":7}),
        )?;
        refused_term(&value)?;
    }
    let mut value = licence_value();
    set(&mut value, "paid", json!(true))?;
    refused_licence(&value)
}

#[test]
fn required_fields_have_no_missing_or_nullable_defaults() -> TestResult {
    for field in [
        "format",
        "id",
        "charter",
        "line",
        "holder",
        "term",
        "operation",
    ] {
        let mut missing = licence_value();
        object(&mut missing)?.remove(field);
        refused_licence(&missing)?;
        let mut null = licence_value();
        set(&mut null, field, Value::Null)?;
        refused_licence(&null)?;
    }
    for field in [
        "format",
        "licence",
        "generation",
        "window",
        "terms",
        "revision",
        "operation",
    ] {
        let mut missing = term_value();
        object(&mut missing)?.remove(field);
        refused_term(&missing)?;
        let mut null = term_value();
        set(&mut null, field, Value::Null)?;
        refused_term(&null)?;
    }
    for field in ["seats", "class", "form", "transferable"] {
        let mut value = term_value();
        object(nested(&mut value, "terms")?)?.remove(field);
        refused_term(&value)?;
    }
    Ok(())
}

#[test]
fn paid_decoders_refuse_owner_dispatch_unknown_versions_and_invalid_ids() -> TestResult {
    for (field, bad) in [
        ("format", json!(2)),
        ("holder", json!(0)),
        ("holder", json!({"Organization":1})),
        ("id", json!("0")),
        ("id", json!("04")),
        ("term", json!(0)),
    ] {
        let mut value = licence_value();
        set(&mut value, field, bad)?;
        refused_licence(&value)?;
    }
    for (field, bad) in [
        ("seats", json!(0)),
        ("seats", json!(1025)),
        ("seats", json!({"kind":"owned_dispatch"})),
        ("class", json!("0")),
        ("transferable", json!(1)),
        ("form", json!({"kind":"none"})),
    ] {
        let mut value = term_value();
        set(nested(&mut value, "terms")?, field, bad)?;
        refused_term(&value)?;
    }
    let mut value = term_value();
    set(&mut value, "format", json!(0))?;
    refused_term(&value)
}

#[test]
fn complete_record_bytes_are_bounded_before_decode() -> TestResult {
    let mut bytes = serde_json::to_vec(&licence_value())?;
    bytes.resize(MAX_PAID_LICENCE_BYTES, b' ');
    require(
        PersonalPaidLicence::decode(&bytes).is_ok(),
        "exact licence bound refused",
    )?;
    bytes.push(b' ');
    require(
        PersonalPaidLicence::decode(&bytes) == Err(PublicationError::Capacity),
        "oversized licence not bounded",
    )?;
    let mut bytes = serde_json::to_vec(&term_value())?;
    bytes.resize(MAX_PAID_TERM_BYTES, b' ');
    require(
        PaidTermRecord::decode(&bytes).is_ok(),
        "exact term bound refused",
    )?;
    bytes.push(b' ');
    require(
        PaidTermRecord::decode(&bytes) == Err(PublicationError::Capacity),
        "oversized term not bounded",
    )?;
    require(PaidTermRecord::decode(&[]).is_err(), "empty term decoded")
}

#[test]
fn local_pointer_check_rejects_crossed_rows_without_claiming_admission() -> TestResult {
    let current = licence(&licence_value())?;
    let base = term_value();
    term(&base)?.matches_current(&current)?;
    for (field, bad) in [
        ("licence", json!("5")),
        ("generation", json!("2")),
        ("revision", json!({"kind":"follow","line":"8"})),
    ] {
        let mut value = base.clone();
        set(&mut value, field, bad)?;
        require(
            term(&value)?.matches_current(&current) == Err(PublicationError::Mismatch),
            "crossed term matched",
        )?;
    }
    // A later transfer changes the current holder/operation, not this term's
    // historical operation or dates. This only proves structural separation.
    let mut transferred = licence_value();
    set(&mut transferred, "holder", json!(8))?;
    set(
        &mut transferred,
        "operation",
        json!(CommitToken::from_bytes([2; 16])),
    )?;
    term(&base)?.matches_current(&licence(&transferred)?)?;
    require(
        serde_json::to_value(term(&base)?)? == base,
        "transfer rewrote immutable terms",
    )
}

#[test]
fn term_and_parent_facts_stay_separate_and_loss_does_not_extend_dates() -> TestResult {
    use crate::commerce::rights::{ParentOperation, ParentRights, check_parent};
    let value = term_value();
    let decoded = term(&value)?;
    let window = decoded.window();
    for (now, phase) in [
        (99, TermPhase::NotEffective),
        (100, TermPhase::Active),
        (200, TermPhase::Grace),
        (210, TermPhase::Lapsed),
    ] {
        require(
            window.phase_at(WorldIcSeconds(now)) == phase,
            "half-open boundary moved",
        )?;
    }
    check_parent(ParentOperation::ExistingPaidSeal, ParentRights::Unavailable)?;
    check_parent(
        ParentOperation::ExistingPaidSource,
        ParentRights::Unavailable,
    )?;
    require(
        check_parent(ParentOperation::NewSale, ParentRights::Unavailable).is_err(),
        "new sale ignored parent loss",
    )?;
    require(
        check_parent(ParentOperation::PaidRenewal, ParentRights::Unavailable).is_err(),
        "renewal ignored parent loss",
    )?;
    check_parent(ParentOperation::PaidRenewal, ParentRights::QualifyingGrant)?;
    require(
        serde_json::to_value(decoded)? == value,
        "policy classification changed dates",
    )
}

#[test]
fn terms_retain_full_identity_width_and_existing_temporal_bounds() -> TestResult {
    let mut value = term_value();
    set(&mut value, "licence", json!(u64::MAX.to_string()))?;
    set(&mut value, "generation", json!(u64::MAX.to_string()))?;
    require(
        serde_json::to_value(term(&value)?)? == value,
        "wide identities narrowed",
    )?;
    // 17g C0: `TermWindow` admits a 365-day owner window, so the 90-day paid
    // bound is the paid record's own: exactly 90 decodes, a second more refuses.
    let mut value = term_value();
    set(
        nested(&mut value, "window")?,
        "term_end",
        json!(100 + MAX_TERM_SECONDS),
    )?;
    set(
        nested(&mut value, "window")?,
        "grace_end",
        json!(100 + MAX_TERM_SECONDS),
    )?;
    term(&value)?;
    let mut value = term_value();
    set(
        nested(&mut value, "window")?,
        "term_end",
        json!(100 + MAX_TERM_SECONDS + 1),
    )?;
    set(
        nested(&mut value, "window")?,
        "grace_end",
        json!(100 + MAX_TERM_SECONDS + 1),
    )?;
    refused_term(&value)?;
    let mut value = term_value();
    set(
        nested(&mut value, "window")?,
        "eligibility_start",
        json!(i64::MIN),
    )?;
    set(nested(&mut value, "window")?, "term_end", json!(i64::MAX))?;
    set(nested(&mut value, "window")?, "grace_end", json!(i64::MAX))?;
    refused_term(&value)
}

#[test]
fn pin_codec_retains_exact_revision_and_refuses_nested_unknown_fields() -> TestResult {
    use crate::commerce::{FiledRevisionId, RevisionIdentity};
    use crate::{Envelope, Revision, RevisionKey, RevisionRef};
    let source = "codec fixture, not an executable or paid grant";
    let envelope = Envelope {
        schema: json!({}),
        presets: json!({}),
        profiles: json!({"Line":{}}),
        rules: json!({}),
        declared_class: "I".into(),
        mind_abi: "fixture=1".into(),
    };
    let revision = Revision {
        format: crate::FORMAT,
        provenance: None,
        reference: RevisionRef {
            key: RevisionKey {
                character: ObjId(7),
                mind: 1,
                number: 1,
            },
            digest: envelope.digest(source)?,
        },
        source: crate::source_digest(source),
        envelope,
        notes: String::new(),
    };
    let pin = RevisionPolicy::Pin {
        filed: FiledRevisionId::new(1)?,
        revision: RevisionIdentity::capture(&revision)?,
    };
    let mut value = term_value();
    set(&mut value, "revision", serde_json::to_value(pin)?)?;
    require(term(&value)?.policy() == pin, "exact pin changed")?;
    // This remains a structural local-pointer check, not proof that the pin
    // belongs to the current charter/line or that its source is authorized.
    term(&value)?.matches_current(&licence(&licence_value())?)?;
    let mut unknown = value.clone();
    set(nested(&mut unknown, "revision")?, "line", json!("3"))?;
    refused_term(&unknown)?;
    let mut unknown = value;
    set(
        nested(nested(&mut unknown, "revision")?, "revision")?,
        "parent_grant",
        json!(true),
    )?;
    refused_term(&unknown)
}
