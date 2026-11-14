#![cfg(test)]

//! Doc 46 CT.0: codec pins for the hull coverage's `record!` types
//! (`subject.rs`) and the consent digest, on plain fixtures shaped like this
//! family's owner fixture (the quote's window bound by `bind_window`). The
//! family gets no walk in CT.0.
use super::super::{
    BIND_ACTION, CLAIM_ACTION, CoverageBasisV1, CoverageConsentV1, CoverageQuoteTermsV1,
    MAX_CONTRACT_BYTES, MAX_CORRELATION_BYTES, MAX_DECISION_BYTES, MAX_HISTORY_BYTES,
    MAX_SUBJECT_BYTES, MAX_TITLE_BYTES, NAMESPACE, RELEASE_ACTION,
};
use super::Result;
use crate::{AccountId, Amount, InstrumentId};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{ClockRef, ClockStamp, DefinitionDigest, DefinitionRef, ObjId};
use mush_org::{OrgId, OrgScopeId};
use mush_testkit::fixtures::token;
use mush_testkit::snapshot::Snapshot;

fn reference(kind: &str, id: &str) -> Result<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new("tacsim")?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            1,
            serde_json::json!({"fixture":"ct0_coverage"}),
        )?,
    )?)
}
fn basis(qualified_role: Option<DefinitionDigest>) -> Result<CoverageBasisV1> {
    let basis = CoverageBasisV1 {
        format: 1,
        product: token(200),
        product_digest: DefinitionDigest::from_bytes([45; 32]),
        quote: token(201),
        quote_digest: DefinitionDigest::from_bytes([46; 32]),
        ic_open: token(202),
        seal_digest: DefinitionDigest::from_bytes([47; 32]),
        admission_digest: DefinitionDigest::from_bytes([48; 32]),
        departure_digest: DefinitionDigest::from_bytes([49; 32]),
        hull: ObjId(3),
        hull_title_revision: 1,
        hull_revision: 0,
        hull_schema: reference("unit", "line_tank")?,
        zone: reference("zone", "urvara_reach")?,
        qualified_role,
    };
    basis.validate()?;
    Ok(basis)
}
fn quote(discount: i64) -> Result<CoverageQuoteTermsV1> {
    let terms = CoverageQuoteTermsV1 {
        format: 1,
        buyer: ObjId(4),
        insurer: OrgId(0),
        scope: OrgScopeId(0),
        treasury: AccountId(3),
        payer: AccountId(1),
        instrument: InstrumentId::new("cr").ok_or("instrument")?,
        adjudicator: ObjId(2),
        insured_value: Amount::from_minor(10000),
        base_premium: Amount::from_minor(100),
        discount: Amount::from_minor(discount),
        premium: Amount::from_minor(100 - discount),
        liability: Amount::from_minor(8000),
        duration_seconds: 100,
        claim_delay_seconds: 10,
        quoted_at_utc: 100,
        expires_at_utc: 160,
    };
    terms.validate()?;
    Ok(terms)
}

#[test]
fn ct0_wire_coverage() -> Result {
    let mut s = Snapshot::new();
    s.section("identities");
    for (label, value) in [
        ("namespace", NAMESPACE),
        ("bind action", BIND_ACTION),
        ("claim action", CLAIM_ACTION),
        ("release action", RELEASE_ACTION),
    ] {
        s.line(label, value);
    }
    for (label, value) in [
        ("max subject bytes", MAX_SUBJECT_BYTES),
        ("max decision bytes", MAX_DECISION_BYTES),
        ("max contract bytes", MAX_CONTRACT_BYTES),
        ("max title bytes", MAX_TITLE_BYTES),
        ("max correlation bytes", MAX_CORRELATION_BYTES),
        ("max history bytes", MAX_HISTORY_BYTES),
    ] {
        s.line(label, &value.to_string());
    }
    let start = ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 1000,
    };
    // The optional qualified role carries the macro's field attribute (an
    // explicit null on the wire): one consent with it, one without.
    let mut consents = Vec::new();
    for (title, role, discount) in [
        (
            "qualified",
            Some(DefinitionDigest::from_bytes([50; 32])),
            10,
        ),
        ("unqualified", None, 0),
    ] {
        let quote = quote(discount)?;
        let consent = CoverageConsentV1 {
            format: 1,
            operation: token(9),
            effect: reference("action", BIND_ACTION)?,
            accepted_at_utc: 110,
            basis: basis(role)?,
            terms: quote.bind_window(start.clone())?,
        };
        consent.validate()?;
        s.section(&format!("digests | {title}"));
        s.wire("consent digest", &consent.digest()?)?;
        consents.push((quote, consent));
    }
    let fail = |json: &mut serde_json::Value| json["format"] = 2.into();
    for (quote, consent) in &consents {
        s.codec("CoverageBasisV1", &consent.basis, fail)?;
        let mut missing = serde_json::to_value(&consent.basis)?;
        if let Some(object) = missing.as_object_mut() {
            object.remove("qualified_role");
        }
        match serde_json::from_value::<CoverageBasisV1>(missing) {
            Ok(_) => s.line("missing qualified_role", "accepted"),
            Err(error) => s.line("missing qualified_role refused", &error.to_string()),
        }
        s.codec("CoverageQuoteTermsV1", quote, fail)?;
        s.codec("CoverageMoneyTermsV1", &consent.terms, fail)?;
        s.codec("CoverageConsentV1", consent, fail)?;
    }
    s.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/wire_coverage.snap")?;
    Ok(())
}
