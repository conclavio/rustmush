//! Doc 46 CT.0: the personal renewal twin's snapshot, on this family's own
//! `Rig`. The walk is shared with the House twin (`ct0_walk.rs`).
use super::{Rig, TestResult};
use crate::EconomyStore;
use crate::managed_renewal as twin;

const PROTOCOL: &str = "personal_mind_renewal_v1";

fn rig() -> TestResult<Box<Rig>> {
    Rig::new()
}
/// The personal twin's every payee role: registry, venue and three royalty
/// parents on a derived lineage (the existing `six_roles` fixture).
fn widen(r: &mut Rig) -> TestResult {
    r.six_roles()
}
fn book(state: &EconomyStore) -> &twin::history::OfferBook {
    &state.renewal_offers
}
fn book_mut(state: &mut EconomyStore) -> &mut twin::history::OfferBook {
    &mut state.renewal_offers
}
fn receipts(fold: &crate::fold::FoldState) -> &twin::receipt::RenewalReceiptAudit {
    &fold.renewal_receipts
}
fn receipts_mut(fold: &mut crate::fold::FoldState) -> &mut twin::receipt::RenewalReceiptAudit {
    &mut fold.renewal_receipts
}

/// The walk both twins share (one source, so not a second `mod` of one file).
mod walk {
    include!("ct0_walk.rs");
}

#[test]
fn ct0_renewal_personal_snapshot() -> TestResult {
    let snap = walk::run()?;
    snap.check(
        env!("CARGO_MANIFEST_DIR"),
        "tests/ct0/renewal_personal.snap",
    )?;
    Ok(())
}
