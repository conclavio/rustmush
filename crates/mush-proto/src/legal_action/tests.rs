use super::*;
use crate::{ClientCommand, InvokeRequest};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const OFFER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn submit(parameters: serde_json::Value) -> serde_json::Value {
    json!({"t":"invoke","d":{"id":7,"request":{"service":"legal_action","request":{
        "op":"submit_self","offer":OFFER,"parameters":parameters
    }}}})
}

#[test]
fn exact_i64_values_survive_public_json_without_javascript_numbers() -> TestResult {
    for (text, expected) in [
        ("-9223372036854775808", i64::MIN),
        ("9223372036854775807", i64::MAX),
        ("0", 0),
    ] {
        let command: ClientCommand =
            serde_json::from_value(submit(json!({"payer":{"kind":"integer","value":text}})))?;
        let ClientCommand::Invoke {
            request: InvokeRequest::LegalAction(LegalActionRequest::SubmitSelf { parameters, .. }),
            ..
        } = command
        else {
            return Err("wrong typed request".into());
        };
        check_eq!(parameters.to_json()?["payer"].as_i64(), Some(expected));
    }
    for bad in [
        "+1",
        "01",
        "-0",
        " 1",
        "1.0",
        "1e1",
        "9223372036854775808",
        "-9223372036854775809",
    ] {
        check!(LegalActionInteger::new(bad.into()).is_err(), "{bad}");
    }
    check!(
        serde_json::from_value::<ClientCommand>(submit(
            json!({"payer":{"kind":"integer","value":9}})
        ))
        .is_err()
    );
    Ok(())
}

#[test]
fn closed_request_refuses_authority_fields_and_nested_unknown_fields() -> TestResult {
    for key in [
        "actor",
        "account",
        "principal",
        "provider",
        "action",
        "definition",
        "idempotency",
        "targets",
        "subject",
    ] {
        let mut wire = submit(json!({}));
        wire["d"]["request"]["request"][key] = json!("injected");
        check!(
            serde_json::from_value::<ClientCommand>(wire).is_err(),
            "{key}"
        );
    }
    check!(
        serde_json::from_value::<ClientCommand>(submit(
            json!({"name":{"kind":"text","value":"name","actor":1}})
        ))
        .is_err()
    );
    for bad in [
        json!(null),
        json!([]),
        json!({"kind":"text","value":[]}),
        json!({"kind":"float","value":1.5}),
    ] {
        check!(serde_json::from_value::<ClientCommand>(submit(json!({"name":bad}))).is_err());
    }
    Ok(())
}

#[test]
fn duplicate_parameters_and_scalar_fields_refuse_before_value_conversion() {
    for raw in [
        r#"{"one":{"kind":"boolean","value":true},"one":{"kind":"boolean","value":false}}"#,
        r#"{"one":{"kind":"boolean","value":true,"value":false}}"#,
        r#"{"one":{"kind":"boolean","kind":"text","value":"x"}}"#,
    ] {
        assert!(serde_json::from_str::<LegalActionParameters>(raw).is_err());
    }
}

#[test]
fn scalar_count_text_and_encoded_byte_limits_are_independent() -> TestResult {
    let value = LegalActionScalar::Text {
        value: "x".repeat(MAX_LEGAL_ACTION_TEXT_BYTES),
    };
    check!(LegalActionParameters::new(BTreeMap::from([("name".into(), value.clone())])).is_ok());
    check!(
        LegalActionParameters::new(BTreeMap::from([(
            "name".into(),
            LegalActionScalar::Text {
                value: "x".repeat(MAX_LEGAL_ACTION_TEXT_BYTES + 1)
            }
        )]))
        .is_err()
    );
    let small = (0..MAX_LEGAL_ACTION_PARAMETERS)
        .map(|i| {
            (
                format!("field_{i}"),
                LegalActionScalar::Boolean { value: true },
            )
        })
        .collect();
    check!(LegalActionParameters::new(small).is_ok());
    let large = (0..=MAX_LEGAL_ACTION_PARAMETERS)
        .map(|i| {
            (
                format!("field_{i}"),
                LegalActionScalar::Boolean { value: true },
            )
        })
        .collect();
    check!(LegalActionParameters::new(large).is_err());
    let encoded = (0..MAX_LEGAL_ACTION_PARAMETERS)
        .map(|i| (format!("field_{i}"), value.clone()))
        .collect();
    check!(LegalActionParameters::new(encoded).is_err());
    for key in ["", ".name", "name.", "a..b", "UPPER", "has-space "] {
        check!(
            LegalActionParameters::new(BTreeMap::from([(
                key.into(),
                LegalActionScalar::Boolean { value: true }
            )]))
            .is_err()
        );
    }
    check!(
        LegalActionParameters::new(BTreeMap::from([(
            "field.name".into(),
            LegalActionScalar::Boolean { value: true }
        )]))
        .is_ok()
    );
    check!(
        LegalActionParameters::new(BTreeMap::from([(
            "x".repeat(97),
            LegalActionScalar::Boolean { value: true }
        )]))
        .is_err()
    );
    Ok(())
}

#[test]
fn tokens_and_status_are_strict_and_carry_no_chosen_actor() -> TestResult {
    let valid = ProjectedOfferId::new(OFFER.into())?;
    check_eq!(
        serde_json::from_str::<ProjectedOfferId>(&serde_json::to_string(&valid)?)?,
        valid
    );
    for bad in [
        OFFER.to_uppercase(),
        "0".repeat(63),
        "g".repeat(64),
        format!(" {OFFER}"),
    ] {
        check!(ProjectedOfferId::new(bad).is_err());
    }
    let reference = "k5:toy:brace:0000000000000007:01010101010101010101010101010101";
    let valid = json!({"op":"status","reference":reference});
    check!(serde_json::from_value::<LegalActionRequest>(valid.clone()).is_ok());
    let mut invalid = valid;
    invalid["actor"] = json!(9);
    check!(serde_json::from_value::<LegalActionRequest>(invalid).is_err());
    check!(
        serde_json::from_value::<LegalActionRequest>(
            json!({"op":"status","reference":"not-a-status"})
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn outcome_variants_are_closed_and_do_not_collapse_ownership_or_absence() -> TestResult {
    let pending = LegalActionResponse::Queued {};
    let unknown = LegalActionResponse::Unavailable { reference: None };
    check_ne!(pending, unknown);
    for state in [
        "queued", "busy", "rejected", "unknown", "expired", "invalid",
    ] {
        check!(serde_json::from_value::<LegalActionResponse>(json!({"state":state})).is_ok());
        check!(
            serde_json::from_value::<LegalActionResponse>(json!({"state":state,"actor":1}))
                .is_err()
        );
    }
    check_eq!(
        serde_json::from_slice::<LegalActionResponse>(&serde_json::to_vec(&unknown)?)?,
        unknown
    );
    Ok(())
}

#[test]
fn raw_websocket_client_command_rejects_duplicate_request_and_parameter_fields() {
    let valid_request = format!(
        r#"{{"service":"legal_action","request":{{"op":"submit_self","offer":"{OFFER}","parameters":{{}}}}}}"#
    );
    let valid = format!(r#"{{"t":"invoke","d":{{"id":7,"request":{valid_request}}}}}"#);
    assert!(serde_json::from_str::<ClientCommand>(&valid).is_ok());
    let duplicate_request = format!(
        r#"{{"t":"invoke","d":{{"id":7,"request":{valid_request},"request":{valid_request}}}}}"#
    );
    assert!(serde_json::from_str::<ClientCommand>(&duplicate_request).is_err());
    for parameters in [
        r#"{"payer":{"kind":"integer","value":"1"},"payer":{"kind":"integer","value":"2"}}"#,
        r#"{"payer":{"kind":"integer","value":"1","value":"2"}}"#,
        r#"{"payer":{"kind":"integer","kind":"text","value":"1"}}"#,
    ] {
        let raw = format!(
            r#"{{"t":"invoke","d":{{"id":7,"request":{{"service":"legal_action","request":{{"op":"submit_self","offer":"{OFFER}","parameters":{parameters}}}}}}}}}"#
        );
        assert!(serde_json::from_str::<ClientCommand>(&raw).is_err());
    }
    for duplicate in [
        r#""op":"submit_self","op":"status""#.to_string(),
        format!(r#""offer":"{OFFER}","offer":"{OFFER}""#),
        r#""parameters":{},"parameters":{}"#.to_string(),
    ] {
        let raw = format!(
            r#"{{"t":"invoke","d":{{"id":7,"request":{{"service":"legal_action","request":{{"op":"submit_self","offer":"{OFFER}","parameters":{{}},{duplicate}}}}}}}}}"#
        );
        assert!(serde_json::from_str::<ClientCommand>(&raw).is_err());
    }
}
