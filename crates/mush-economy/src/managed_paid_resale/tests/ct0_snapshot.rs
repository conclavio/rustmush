//! Doc 46 CT.0: codec pins for the paid resale's `closed!` types
//! (`subject.rs`) and its hand-written strict terms pair, on this family's own
//! `sample()` consent (Pending) and its Immediate twin, and the digests and
//! identity strings they carry. The family gets no walk in CT.0.
use super::super::{
    ACCEPT, FUND, MAX_CONTRACT_BYTES, MAX_CORRELATION_BYTES, MAX_HISTORY_BYTES,
    MAX_HOLDER_GENERATION, MAX_OFFER_SECONDS, MAX_SUBJECT_BYTES, NAMESPACE, OFFER, REFUND, SETTLE,
    SETTLEMENT_IC_SECONDS,
};
use super::{PaidResaleModeV1, Result, action, sample};
use mush_testkit::snapshot::Snapshot;

#[test]
fn ct0_wire_paid_resale() -> Result {
    let pending = sample()?;
    let mut immediate = pending.clone();
    immediate.mode = PaidResaleModeV1::Immediate;
    immediate.effect = action(ACCEPT)?;
    immediate.claims.open_paid = 0;
    immediate.terms.venue = None;
    immediate.validate()?;

    let mut s = Snapshot::new();
    s.section("identities");
    for (label, value) in [
        ("namespace", NAMESPACE),
        ("offer action", OFFER),
        ("accept action", ACCEPT),
        ("fund action", FUND),
        ("settle action", SETTLE),
        ("refund action", REFUND),
    ] {
        s.line(label, value);
    }
    for (label, value) in [
        ("max subject bytes", MAX_SUBJECT_BYTES),
        ("max correlation bytes", MAX_CORRELATION_BYTES),
        ("max contract bytes", MAX_CONTRACT_BYTES),
        ("max history bytes", MAX_HISTORY_BYTES),
    ] {
        s.line(label, &value.to_string());
    }
    for (label, value) in [
        ("max offer seconds", MAX_OFFER_SECONDS),
        ("settlement seconds", SETTLEMENT_IC_SECONDS),
    ] {
        s.line(label, &value.to_string());
    }
    s.line("max holder generation", &MAX_HOLDER_GENERATION.to_string());

    for (title, consent) in [("pending", &pending), ("immediate", &immediate)] {
        s.section(&format!("digests | {title}"));
        s.wire("consent digest", &consent.digest()?)?;
        s.wire("deadline", &consent.deadline)?;
        let payouts = consent.terms.payouts(consent.payer)?;
        s.line("payouts", &format!("{}", payouts.len()));
    }

    let basis = &pending.terms.basis;
    // A birth reference is three fixed-width fields: no JSON edit can carry it
    // past its 1024-byte bound, the only rule it holds.
    s.codec_shape("PaidBirthRefV1", &basis.birth)?;
    s.codec("PaidHolderRefV1", &basis.holder, |json| {
        json["generation"] = 0.into();
    })?;
    s.codec("PaidResaleWindowV1", &basis.term.window, |json| {
        json["format"] = 2.into();
    })?;
    s.codec("PaidResaleTermBasisV1", &basis.term, |json| {
        json["generation"] = 0.into();
    })?;
    s.codec("PaidResaleBasisV1", basis, |json| {
        json["licence"] = 0.into();
    })?;
    s.codec("PaidResaleClaimsV1", &pending.claims, |json| {
        json["next_transition"] = 0.into();
    })?;
    for consent in [&pending, &immediate] {
        // The terms pair is hand-written strict (explicit-null venue): one pin
        // with the venue present and one with it absent.
        s.codec("PaidResaleTermsV1", &consent.terms, |json| {
            json["format"] = 2.into();
        })?;
        s.codec("PaidResaleConsentV1", consent, |json| {
            json["format"] = 2.into();
        })?;
    }
    let mut missing = serde_json::to_value(&immediate.terms)?;
    if let Some(object) = missing.as_object_mut() {
        object.remove("venue");
    }
    match serde_json::from_value::<super::PaidResaleTermsV1>(missing) {
        Ok(_) => s.line("missing venue", "accepted"),
        Err(error) => s.line("missing venue refused", &error.to_string()),
    }
    s.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/wire_paid_resale.snap",
    )?;
    Ok(())
}
