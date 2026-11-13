#![cfg(test)]

//! Doc 46 CT.0: codec pins of the House acquisition economics (`data!`) with
//! every digest over them, for both House protocols
//! (`tests/ct0/wire_house_purchase_economics.snap`).
use super::*;
use crate::commerce::tests::ct0_rig as rig;
use mush_testkit::snapshot::Snapshot;

#[test]
fn ct0_house_purchase_economics_wire() -> TestResult {
    let (_engine, f) = rig::families()?;
    let mut s = Snapshot::new();
    s.section("constants");
    s.line(
        "MAX_ECONOMICS_BYTES",
        &crate::commerce::house_purchase::economics::MAX_ECONOMICS_BYTES.to_string(),
    );
    rig::family(&mut s, &f.house_economics);
    for protocol in [HouseProtocol::HouseV1, HouseProtocol::HouseV2] {
        s.section(&format!("{protocol:?}"));
        s.line("format", &protocol.format().to_string());
        s.line("version", &protocol.version().to_string());
        s.line("seats", &protocol.seats().to_string());
        let term = row(2, 3, protocol)?;
        rig::document(&mut s, "term economics", &f.house_economics, &term)?;
        s.wire("term economics digest", &term.digest()?)?;
        s.wire("grant digest", &term.grant().digest()?)?;
        s.codec(
            &format!("HouseRenewalTemplateV1 ({protocol:?})"),
            term.grant().renewal(),
            |j| j["format"] = 3.into(),
        )?;
        s.codec(
            &format!("HouseGrantEconomicsV1 ({protocol:?})"),
            term.grant(),
            |j| j["format"] = 3.into(),
        )?;
        s.codec(
            &format!("HouseTermEconomicsV1 ({protocol:?})"),
            &term,
            |j| j["format"] = 3.into(),
        )?;
    }
    s.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/wire_house_purchase_economics.snap",
    )?;
    Ok(())
}
