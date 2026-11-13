//! Doc 46 CT.0: codec pins for the pending charter transfer's `closed!` types
//! (`subject.rs`), on this family's own `sample()` consent, and the digests
//! and identity strings they carry. The family gets no walk in CT.0.
use super::super::{
    FUND_ACTION, MAX_CHARTER_OWNERSHIP_GENERATION, MAX_CHARTER_PENDING_OFFER_SECONDS,
    MAX_CHARTER_PENDING_SUBJECT_BYTES, MAX_CHARTER_SETTLEMENT_IC_SECONDS,
    MAX_PENDING_CONTRACT_BYTES, MAX_PENDING_CORRELATION_BYTES, MAX_PENDING_HISTORY_BYTES,
    NAMESPACE, OFFER_ACTION, REFUND_ACTION, SETTLE_ACTION,
};
use super::{Result, sample};
use mush_testkit::snapshot::Snapshot;

#[test]
fn ct0_wire_charter_transfer() -> Result {
    let consent = sample()?;
    let mut s = Snapshot::new();
    s.section("identities");
    for (label, value) in [
        ("namespace", NAMESPACE),
        ("offer action", OFFER_ACTION),
        ("fund action", FUND_ACTION),
        ("settle action", SETTLE_ACTION),
        ("refund action", REFUND_ACTION),
    ] {
        s.line(label, value);
    }
    for (label, value) in [
        ("max subject bytes", MAX_CHARTER_PENDING_SUBJECT_BYTES),
        ("max contract bytes", MAX_PENDING_CONTRACT_BYTES),
        ("max correlation bytes", MAX_PENDING_CORRELATION_BYTES),
        ("max history bytes", MAX_PENDING_HISTORY_BYTES),
    ] {
        s.line(label, &value.to_string());
    }
    for (label, value) in [
        ("max offer seconds", MAX_CHARTER_PENDING_OFFER_SECONDS),
        ("max settlement seconds", MAX_CHARTER_SETTLEMENT_IC_SECONDS),
    ] {
        s.line(label, &value.to_string());
    }
    s.line(
        "max ownership generation",
        &MAX_CHARTER_OWNERSHIP_GENERATION.to_string(),
    );
    s.section("digests");
    s.wire("terms digest", &consent.terms.digest()?)?;
    s.wire("consent digest", &consent.digest()?)?;
    s.wire("deadline", &consent.deadline)?;

    s.codec(
        "CharterOwnershipBasisV1",
        &consent.terms.ownership,
        |json| json["generation"] = 0.into(),
    )?;
    s.codec("CharterTermBasisV1", &consent.terms.owner_term, |json| {
        json["licence"] = 0.into();
    })?;
    s.codec("CharterPendingTermsV1", &consent.terms, |json| {
        json["format"] = 2.into();
    })?;
    s.codec("CharterPendingConsentV1", &consent, |json| {
        json["format"] = 2.into();
    })?;
    s.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/wire_charter_transfer.snap",
    )?;
    Ok(())
}
