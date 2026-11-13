//! Doc 46 CT.0: codec pins of the acquisition economics (`record!`) with every
//! digest over them (`tests/ct0/wire_entitlement_economics.snap`).
use super::*;
use crate::commerce::tests::ct0_rig::{self as rig};
use mush_testkit::snapshot::Snapshot;

#[test]
fn ct0_entitlement_economics_wire() -> TestResult {
    let (_engine, f) = rig::families()?;
    let mut s = Snapshot::new();
    s.section("constants");
    s.line("ECONOMICS_FORMAT", &ECONOMICS_FORMAT.to_string());
    s.line("MAX_ECONOMICS_BYTES", &MAX_ECONOMICS_BYTES.to_string());
    s.line("MAX_ROYALTY_PERMILLE", &MAX_ROYALTY_PERMILLE.to_string());
    rig::family(&mut s, &f.term_economics);
    rig::family(&mut s, &f.filed_royalties);

    s.section("rows and digests");
    let chain = chain()?;
    rig::document(&mut s, "filed royalty terms", &f.filed_royalties, &chain)?;
    s.wire("filed royalty digest", &chain.digest()?)?;
    s.wire("filed royalty reference", &chain.reference()?)?;
    let sealed = grant(PaidForm::Sealed {})?;
    let source = grant(PaidForm::OpenResale {})?;
    s.wire("sealed grant digest", &sealed.digest()?)?;
    s.wire("source grant digest", &source.digest()?)?;
    let term = term()?;
    rig::document(&mut s, "term economics", &f.term_economics, &term)?;
    s.wire("term economics digest", &term.digest()?)?;
    let mut legacy = edge(1, 0)?;
    legacy.basis = RoyaltyBasisV1::LegacySourceFormsV2Zero {
        effect: definition("action", "purchase_original_mind", 2)?,
    };
    legacy.validate()?;

    let source_rate = edge(1, 100)?;
    s.codec("RenewalTemplateV1", source.renewal(), |j| {
        j["format"] = 2.into()
    })?;
    s.codec("GrantEconomicsV1 (sealed)", &sealed, |j| {
        j["format"] = 2.into()
    })?;
    s.codec("GrantEconomicsV1 (source)", &source, |j| {
        j["downstream"]["royalty_permille"] = 301.into()
    })?;
    s.codec("PaidTermEconomicsV1", &term, |j| j["format"] = 2.into())?;
    s.codec("RoyaltyEdgeV1 (explicit)", &source_rate, |j| {
        j["royalty_permille"] = 301.into()
    })?;
    s.codec("RoyaltyEdgeV1 (legacy zero)", &legacy, |j| {
        j["royalty_permille"] = 1.into()
    })?;
    s.codec("FiledRoyaltyTermsV1", &chain, |j| j["format"] = 2.into())?;
    s.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/wire_entitlement_economics.snap",
    )?;
    Ok(())
}
