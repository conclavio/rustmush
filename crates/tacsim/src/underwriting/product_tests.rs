use super::*;
use mush_core::{
    DefinitionDigest, DefinitionId, DefinitionKey, DefinitionKind, DefinitionRef, ObjId, OwnerId,
};
use mush_economy::{AccountId, Amount, InstrumentId};
use mush_org::{OrgId, OrgScopeId};
use mush_roll::commerce::{CharterId, FiledRevisionId, LineId, RevisionIdentity};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn digest(byte: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([byte; 32])
}
fn reference(id: &str, byte: u8) -> Result<DefinitionRef, Box<dyn std::error::Error>> {
    Ok(DefinitionRef::new(
        DefinitionKey::new(
            OwnerId::new("tacsim")?,
            DefinitionKind::new("coverage_fixture")?,
            DefinitionId::new(id)?,
        ),
        1,
        digest(byte),
    )?)
}
pub(super) fn product() -> Result<CoverageProductV1, Box<dyn std::error::Error>> {
    // Structural fixtures only: these values cannot issue a quote, establish
    // certification, debit a premium, reserve exposure or activate a policy.
    let revision: RevisionIdentity = serde_json::from_value(json!({
        "format":1,"revision":{"key":{"character":10,"mind":1,"number":1},
        "digest":digest(1)},"source":digest(2)
    }))?;
    let approval = ApprovedConductV1::new(
        CharterId::new(20)?,
        LineId::new(1)?,
        FiledRevisionId::new(1)?,
        revision,
        digest(3),
        digest(4),
        SafeConductRole::Escort,
        "Escort".into(),
    )?;
    Ok(CoverageProductV1::new(
        OrgId(1),
        OrgScopeId(2),
        AccountId(3),
        ObjId(4),
        CoverageTermsV1::launch(
            InstrumentId::new("tacsim.credit").ok_or("currency")?,
            Amount::from_minor(100),
            Amount::from_minor(100_000_000),
            7 * 86_400,
        )?,
        vec![reference("carrier", 5)?],
        vec![reference("corridor", 6)?],
        vec![approval],
        600,
    )?)
}

#[test]
fn underwriting_product_prices_use_registered_credit_minor_units() -> TestResult {
    let product = product()?;
    // CREDIT scale is two: 10,000cr of agreed value costs 100cr normally,
    // conditionally 90cr with qualified Safe Conduct, and reserves 8,000cr.
    let example = product.illustrate(Amount::from_minor(1_000_000))?;
    assert_eq!(product.terms().instrument().as_str(), "tacsim.credit");
    assert_eq!(example.ordinary().premium(), Amount::from_minor(10_000));
    assert_eq!(
        example.with_qualified_safe_conduct().premium(),
        Amount::from_minor(9_000)
    );
    assert_eq!(
        example.with_qualified_safe_conduct().maximum_payout(),
        Amount::from_minor(800_000)
    );
    assert_eq!(
        product.loss_rule(),
        LossRule::RecordedIcDestructionWithWreckTitle
    );
    assert_eq!(product.quote_seconds(), 600);
    Ok(())
}

#[test]
fn underwriting_product_digest_pins_every_issuer_term_and_approval_field() -> TestResult {
    let original = product()?;
    let value = serde_json::to_value(&original)?;
    assert_eq!(
        CoverageProductV1::decode(&serde_json::to_vec(&original)?)?,
        original
    );
    for (pointer, replacement) in [
        ("/insurer", json!(2)),
        ("/scope", json!(3)),
        ("/treasury", json!(4)),
        ("/adjudicator", json!(5)),
        ("/quote_seconds", json!(601)),
        ("/terms/premium_permille", json!(11)),
        ("/terms/discount_permille", json!(101)),
        ("/terms/payout_permille", json!(801)),
        ("/terms/term_seconds", json!(86400)),
        ("/terms/claim_delay_seconds", json!(1)),
        ("/terms/maximum_value", json!(100_000_001)),
        ("/hulls/0/digest", json!(digest(7))),
        ("/zones/0/version", json!(2)),
        ("/approvals/0/charter", json!("21")),
        ("/approvals/0/line", json!("2")),
        ("/approvals/0/filed", json!("2")),
        ("/approvals/0/revision/revision/digest", json!(digest(8))),
        ("/approvals/0/revision/source", json!(digest(9))),
        ("/approvals/0/certification", json!(digest(10))),
        ("/approvals/0/role_trial", json!(digest(11))),
        ("/approvals/0/role", json!("transit")),
        ("/approvals/0/profile", json!("Transit")),
    ] {
        let mut altered = value.clone();
        *altered.pointer_mut(pointer).ok_or("fixture path")? = replacement;
        let altered: CoverageProductV1 = serde_json::from_value(altered)?;
        assert_ne!(altered.digest()?, original.digest()?);
    }
    Ok(())
}

#[test]
fn underwriting_product_rejects_unbounded_ambiguous_and_unknown_declarations() -> TestResult {
    let value = serde_json::to_value(product()?)?;
    for (pointer, replacement) in [
        ("/format", json!(2)),
        ("/adjudicator", json!(0)),
        ("/quote_seconds", json!(0)),
        ("/quote_seconds", json!(3601)),
        ("/loss_rule", json!("any_journey_failure")),
        ("/hulls", json!([])),
        ("/zones", json!([])),
        ("/hulls/0/digest", json!(digest(0))),
        ("/approvals/0/format", json!(2)),
        ("/approvals/0/certification", json!(digest(0))),
        ("/approvals/0/role_trial", json!(digest(0))),
        ("/approvals/0/profile", json!("")),
        ("/approvals/0/profile", json!(" Escort")),
        ("/approvals/0/profile", json!("Esc\nort")),
        ("/approvals/0/profile", json!("x".repeat(65))),
        ("/approvals/0/role", json!("combat")),
    ] {
        let mut altered = value.clone();
        *altered.pointer_mut(pointer).ok_or("fixture path")? = replacement;
        assert!(serde_json::from_value::<CoverageProductV1>(altered).is_err());
    }
    for field in ["hulls", "zones", "approvals"] {
        let mut altered = value.clone();
        let item = altered[field][0].clone();
        altered[field] = Value::Array(vec![item; 33]);
        assert!(serde_json::from_value::<CoverageProductV1>(altered).is_err());
    }
    let mut unknown = value.clone();
    unknown["eligible"] = json!(true);
    assert!(serde_json::from_value::<CoverageProductV1>(unknown).is_err());
    let mut unknown = value;
    unknown["approvals"][0]["eligible"] = json!(true);
    assert!(serde_json::from_value::<CoverageProductV1>(unknown).is_err());
    assert_eq!(
        CoverageProductV1::decode(&vec![b' '; CoverageProductV1::MAX_BYTES + 1]),
        Err(CoverageError::Bound)
    );
    assert_eq!(CoverageProductV1::decode(&[]), Err(CoverageError::Bound));
    Ok(())
}

#[test]
fn underwriting_product_refuses_duplicate_conflicting_profile_approvals() -> TestResult {
    let original = serde_json::to_value(product()?)?;
    let mut duplicate = original.clone();
    let first = duplicate["approvals"][0].clone();
    duplicate["approvals"]
        .as_array_mut()
        .ok_or("approval list")?
        .push(first);
    assert!(serde_json::from_value::<CoverageProductV1>(duplicate.clone()).is_err());
    duplicate["approvals"][1]["role"] = json!("transit");
    duplicate["approvals"][1]["role_trial"] = json!(digest(12));
    assert!(serde_json::from_value::<CoverageProductV1>(duplicate.clone()).is_err());
    duplicate["approvals"][1]["profile"] = json!("Transit");
    assert!(serde_json::from_value::<CoverageProductV1>(duplicate).is_ok());
    let mut hulls = original;
    let first = hulls["hulls"][0].clone();
    hulls["hulls"]
        .as_array_mut()
        .ok_or("hull list")?
        .push(first);
    assert!(serde_json::from_value::<CoverageProductV1>(hulls).is_err());
    Ok(())
}

#[test]
fn underwriting_product_accepts_real_zero_based_org_scope_and_account_ids() -> TestResult {
    let mut wire = serde_json::to_value(product()?)?;
    for field in ["insurer", "scope", "treasury"] {
        wire[field] = json!(0);
    }
    let product: CoverageProductV1 = serde_json::from_value(wire)?;
    assert_eq!(product.insurer(), OrgId(0));
    assert_eq!(product.scope(), OrgScopeId(0));
    assert_eq!(product.treasury(), AccountId(0));
    assert_eq!(
        CoverageProductV1::decode(&serde_json::to_vec(&product)?)?,
        product
    );
    Ok(())
}

#[test]
fn underwriting_ordinary_product_accepts_empty_approvals_without_qualification() -> TestResult {
    let original = product()?;
    let mut value = serde_json::to_value(&original)?;
    value["approvals"] = json!([]);
    let ordinary = CoverageProductV1::decode(&serde_json::to_vec(&value)?)?;
    assert!(ordinary.approvals().is_empty());
    assert_ne!(ordinary.digest()?, original.digest()?);
    assert_eq!(
        ordinary
            .illustrate(Amount::from_minor(1_000_000))?
            .ordinary()
            .premium(),
        Amount::from_minor(10_000)
    );
    // The conditional illustration is arithmetic only. No actual approved
    // selection exists, and the ordinary producer never accepts one.
    assert_eq!(serde_json::to_value(&ordinary)?, value);
    assert_eq!(
        CoverageProductV1::decode(&serde_json::to_vec(&original)?)?,
        original
    );
    Ok(())
}
