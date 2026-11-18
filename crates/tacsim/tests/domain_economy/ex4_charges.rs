//! **Doc 48 cut EX.4 — fees and obligations at 24:1** (Q-48-8, -15, -54, -55, -56), over the
//! composed lane (`driving`'s rig) at the live rate, 24:1, where an IC day is one real hour.
//!
//! 1. **A week offline costs a hull 840 cr as arrears, not Delinquent**, and she still undocks.
//! 2. **The fees land where they should, once, and the lien collects at her next sale.**
//! 3. **The Co-op share applies on a personal chartered sale.**
//!
//! (The locker's charge being collected is EX.1b's locker story, `ex1b_venue`.)
//!
//! **What is a fixture.** The captains' money (minted by staff, and staff taking a broke owner's
//! remainder away); the lot seeded into a hold; the cargo loaded for the passage. Every act of the
//! captains' is the production path, and the clock runs at the live rate.

use mush_core::{GOD, ObjId};
use mush_economy::AccountId;
use mush_inventory::LotId;
use mush_org::PrincipalId;

use crate::driving_rig::{
    Rig, board, hold_vault_of, load, port_map, run, seed_lot, set_rate, stocked, sweep,
};
use crate::hull_resale_ex2::{
    accept, account_of, balance, captain, exchange, list, mint, owner_of_her, quote,
};

/// An IC day at 24:1, real seconds.
const IC_DAY_REAL: i64 = 3_600;

/// The port authority's account at Ceres (the venue's operator).
fn authority(rig: &Rig) -> AccountId {
    let map = port_map(&rig.tw, "ceres_port");
    let row = map
        .orgs
        .iter()
        .find(|(id, _)| id.as_str() == "freeport_registry")
        .map(|(_, row)| row.clone())
        .expect("Ceres' authority");
    AccountId(row.account.expect("its account"))
}

/// Ceres' treasury (the levy's sink).
fn treasury(rig: &Rig) -> AccountId {
    AccountId(
        port_map(&rig.tw, "ceres_port")
            .treasury
            .expect("a treasury"),
    )
}

/// Staff take `who`'s whole balance away: an owner who cannot pay.
fn broke(rig: &mut Rig, who: ObjId) {
    let account = account_of(rig, who);
    let left = balance(rig, account);
    let sink = treasury(rig);
    if left > 0 {
        let out = run(
            &mut rig.tw,
            GOD,
            &format!("@econ/pay {}={} {left}", account.0, sink.0),
        );
        assert!(!out.contains("Usage"), "{out}");
    }
    assert_eq!(balance(rig, account), 0);
}

/// `hull`'s berth at Ceres, as `who` owes it.
fn berth(rig: &Rig, who: ObjId, hull: ObjId) -> Option<mush_economy::Obligation> {
    let ns = tacsim::port::berth::berth_namespace("ceres_port", hull);
    mush_economy::player_obligations_of(rig.tw.world(), &PrincipalId::Object(who), &ns)
        .into_iter()
        .find(|o| o.schedule.cadence.is_some())
}

fn arrears(rig: &Rig, who: ObjId, hull: ObjId) -> i64 {
    let ns = tacsim::port::berth::berth_namespace("ceres_port", hull);
    tacsim::port::berth::arrears_of(rig.tw.world(), &PrincipalId::Object(who), &ns)
}

/// One IC day at 24:1: every tick fires once.
fn day(rig: &mut Rig) {
    sweep(&mut rig.tw, IC_DAY_REAL);
}

/// An owner of a bought-out Dray at Ceres, broke, at 24:1, her berth open.
fn broke_owner(rig: &mut Rig) -> (ObjId, ObjId, tacsim::port::HullRow) {
    let (who, row) = owner_of_her(rig, "Owner", 1, "Salt Moth");
    let hull = ObjId(row.hull);
    set_rate(&mut rig.tw, 24);
    broke(rig, who);
    sweep(&mut rig.tw, 60);
    let open = berth(rig, who, hull).expect("her berth is charged");
    assert_eq!(open.amount.minor(), 500, "5 cr an IC day");
    assert!(
        open.grace.allowance() >= mush_economy::PLAYER_OWED_MIN_GRACE,
        "at least 28 periods of grace"
    );
    (who, hull, row)
}

/// **A week offline costs her 840 cr as arrears, not Delinquent; and she undocks.**
#[test]
fn ex4_a_week_offline_costs_a_hull_840_cr_as_arrears_and_she_still_undocks() {
    let mut rig = stocked();
    let (who, hull, row) = broke_owner(&mut rig);
    for _ in 0..200 {
        if berth(&rig, who, hull).is_some_and(|o| o.missed_total >= 168) {
            break;
        }
        day(&mut rig);
    }
    let o = berth(&rig, who, hull).expect("the berth");
    assert_eq!(o.missed_total, 168, "a real week at 24:1");
    assert!(
        matches!(o.status, mush_economy::ObligationStatus::Grace { .. }),
        "not delinquent: {:?}",
        o.status
    );
    assert_eq!(arrears(&rig, who, hull), 84_000, "840 cr in arrears");
    // Nothing holds her at the berth: with her arrears unpaid she loads and sails.
    load(&mut rig, &row, 60, 20, 98_600);
    board(&mut rig, who);
    let (said, queued) = crate::driving_rig::command(&mut rig, who, "+ship/undock stickney");
    assert!(
        said.contains("Clearance filed for Stickney Yards"),
        "{said}"
    );
    // The port's own ticks ride with her clearance (EX.3a's board purchase among them, which
    // a long stride can stage twice), so they land quietly; that she sails is asserted below.
    crate::driving_rig::land_quietly(&mut rig, &queued);
    assert_eq!(arrears(&rig, who, hull), 84_000, "her arrears stand");
    // Away from the berth, the berth ends; the arrears are the lien.
    for _ in 0..48 {
        if matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::UnderWay
        ) {
            break;
        }
        sweep(&mut rig.tw, 600);
        // As above: the port's ticks land quietly.
        let queued = crate::driving_rig::drain(&mut rig.tw);
        crate::driving_rig::land_quietly(&mut rig, &queued);
    }
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), hull),
            tacsim::port::ship::Berth::UnderWay
        ),
        "she sailed"
    );
    sweep(&mut rig.tw, 60);
    assert!(berth(&rig, who, hull).is_none(), "no berth while under way");
}

/// **The fees land where they should, once, and the lien collects at her next sale at the port.**
#[test]
fn ex4_fees_land_once_and_the_lien_collects_at_her_next_sale() {
    let mut rig = stocked();
    let (who, hull, _) = broke_owner(&mut rig);
    for _ in 0..10 {
        day(&mut rig);
    }
    let owed = arrears(&rig, who, hull);
    assert!(owed >= 4_500, "ten days unpaid: {owed}");
    let (buyer, _) = captain(&mut rig, "Buyer", 2, "Brine Lark");
    mint(&mut rig, buyer, 7_000_000);
    let (operator, sink) = (authority(&rig), treasury(&rig));
    let (op0, sink0) = (balance(&rig, operator), balance(&rig, sink));
    // The listing fee: 1% of 60,000 cr, paid once at posting from the seller's own account.
    mint(&mut rig, who, 60_000);
    let listing = list(&mut rig, who, hull);
    assert_eq!(
        balance(&rig, account_of(&rig, who)),
        0,
        "the fee took it all"
    );
    assert_eq!(balance(&rig, operator) - op0, 60_000, "the listing fee");
    for _ in 0..3 {
        day(&mut rig);
    }
    let fees = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy")
        .transactions()
        .filter(|t| {
            t.description
                .contains(&format!("market listing #{} fee", listing.0))
        })
        .count();
    assert_eq!(fees, 1, "charged once a listing");
    let owed = arrears(&rig, who, hull);
    let (op1, sink1) = (balance(&rig, operator), balance(&rig, sink));
    // The sale: the seller paid 96% less her arrears, the operator 3%, the sink 1%.
    let (said, q) = quote(&mut rig, buyer, listing);
    let q = q.unwrap_or_else(|| panic!("no quote: {said}"));
    let (said, contract) = accept(&mut rig, buyer, q);
    assert!(contract.is_some(), "{said}");
    sweep(&mut rig.tw, 31);
    assert_eq!(
        tacsim::port::house_sweep::title_of(rig.tw.world(), hull),
        Some(PrincipalId::Object(buyer)),
        "sold"
    );
    assert_eq!(
        balance(&rig, sink) - sink1,
        60_000,
        "the 1% levy to the treasury"
    );
    assert_eq!(
        balance(&rig, operator) - op1,
        180_000 + owed,
        "the 3% cut and the berth's arrears to the authority"
    );
    assert_eq!(arrears(&rig, who, hull), 0, "the lien collected");
    assert_eq!(
        balance(&rig, account_of(&rig, who)),
        6_000_000 - 180_000 - 60_000 - owed,
        "the seller's net"
    );
    let _ = (op0, sink0);
}

/// **The Co-op's share on a personal ask from a chartered hold** (Q-48-56).
#[test]
fn ex4_the_coop_share_applies_on_a_personal_chartered_sale() {
    let mut rig = stocked();
    let (seller, row) = captain(&mut rig, "Charter", 3, "Tern");
    let hold = hold_vault_of(&rig.tw, &row);
    seed_lot(&mut rig, hold, "metals", 2, LotId(0xe4_01));
    mint(&mut rig, seller, 100_000);
    let (buyer, _) = captain(&mut rig, "Buyer", 4, "Brine Lark");
    mint(&mut rig, buyer, 1_000_000);
    let trust = AccountId(
        port_map(&rig.tw, "ceres_port")
            .orgs
            .iter()
            .find(|(id, _)| id.as_str() == "coop_trust")
            .and_then(|(_, row)| row.account)
            .expect("the Trust's account"),
    );
    exchange(&mut rig, seller);
    let before = balance(&rig, account_of(&rig, seller));
    let said = run(
        &mut rig.tw,
        seller,
        &format!("+market/post {} price=30000", LotId(0xe4_01)),
    );
    assert!(said.contains("Listed for sale as #"), "{said}");
    assert_eq!(
        before - balance(&rig, account_of(&rig, seller)),
        1_000,
        "the listing fee's 10 cr floor (1% of 600 cr is less)"
    );
    // The Trust's own hull at its home port owes no berth (Q-48-55).
    let hull = ObjId(row.hull);
    let trust_org = port_map(&rig.tw, "ceres_port")
        .orgs
        .iter()
        .find(|(id, _)| id.as_str() == "coop_trust")
        .map(|(_, row)| row.org)
        .expect("the Trust");
    let ns = tacsim::port::berth::berth_namespace("ceres_port", hull);
    assert!(
        mush_economy::player_obligations_of(
            rig.tw.world(),
            &PrincipalId::Organization(mush_org::OrgId(trust_org)),
            &ns
        )
        .is_empty(),
        "a House's hull at home owes no berth"
    );
    let listing = said
        .split("Listed for sale as #")
        .nth(1)
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse::<u64>().ok())
        .map(mush_market::ListingId)
        .expect("a listing id");
    let queued = crate::driving_rig::drain(&mut rig.tw);
    crate::driving_rig::land_all(&mut rig, &queued);
    let (trust0, seller0) = (
        balance(&rig, trust),
        balance(&rig, account_of(&rig, seller)),
    );
    exchange(&mut rig, buyer);
    let said = run(&mut rig.tw, buyer, &format!("+market/quote {listing}=2"));
    let q = crate::hull_resale_ex2::newest(&rig, buyer).unwrap_or_else(|| panic!("{said}"));
    let said = run(&mut rig.tw, buyer, &format!("+market/accept {q}"));
    let queued = crate::driving_rig::drain(&mut rig.tw);
    crate::driving_rig::land_all(&mut rig, &queued);
    for _ in 0..4 {
        sweep(&mut rig.tw, 31);
        let queued = crate::driving_rig::drain(&mut rig.tw);
        crate::driving_rig::land_all(&mut rig, &queued);
    }
    let share = balance(&rig, trust) - trust0;
    let net = balance(&rig, account_of(&rig, seller)) - seller0;
    assert!(share > 0, "the Co-op took its share: {said}");
    // 2 t at 300 cr: 3% cut and 1% levy (1,800 + 600 minor), the share, the rest the seller's.
    assert_eq!(
        net + share + 1_800 + 600,
        60_000,
        "share {share}, net {net}: {said}"
    );
}
