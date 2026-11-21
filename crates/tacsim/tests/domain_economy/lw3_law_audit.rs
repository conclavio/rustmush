//! **Doc 50 unit LW.3 — the port audit, the receipt of a private sale, and the party who reports
//! it** (Q-50-35, Q-50-36, Q-50-43, Q-50-65, Q-50-66, Q-50-67), over the composed lane
//! (`driving`'s stocked rig), through registered verbs and the port tick.
//!
//! - **The audit** (`lw3_an_audit_finds_strategic_ore_held_without_the_papers`): a prospector
//!   docked at Ceres holds 6 t of strategic ore with a license and no claim. Staff order an audit
//!   (`+law/audit #<hull>`): it finds the ore, records an offense of possession under the
//!   Freeport, and fines it at once by the tonne (3,000 cr), paid from the prospector's account.
//!   With a claim on a strategic rock as well, the next audit finds nothing.
//! - **The report** (`lw3_a_buyer_reports_a_private_sale_hands_over_the_lot_and_is_repaid`): a
//!   prospector with no claim sells a trader 2 t of strategic ore privately. Settlement writes a
//!   receipt for each party, recording that the seller lacked the papers. The trader reports it
//!   (`+law/report <quote> go`): the lot leaves the trader's locker for the Registry's vault in
//!   the report's own action; the tick records the sale against the seller, fines the seller
//!   (1,000 cr, paid at once); and once the fine is collected the trader is paid the price paid
//!   (600 cr) and a fifth of the fine (200 cr).
//!
//! **What is a fixture.** The ore, seeded; the claim on a strategic rock, a typed record; money.

use mush_core::{GOD, ObjId};
use mush_economy::VaultId;
use mush_inventory::LotId;
use mush_org::PrincipalId;
use tacsim::offense::{Evidence, Offender, OffenseKey, OffenseKind, Proof};

use crate::driving_rig::{
    cid, drain, hold_vault_of, land_all, pilot, prospector, run, seed_lot, seed_pool, seed_typed,
    station_room, teleport,
};

const CERES: &str = "ceres_port";

/// A live registered claim on the first strategic rock, held by `who` (a typed record).
pub(crate) fn seed_strategic_claim(rig: &mut crate::driving_rig::Rig, who: ObjId) {
    let (cluster, letter) = tacsim::space::mining::all_orbital_clusters(rig.tw.world())
        .into_iter()
        .find_map(|cluster| {
            cluster
                .rocks
                .iter()
                .find(|rock| {
                    rock.spectral
                        .families()
                        .contains(&tacsim::surface::OreFamily::StrategicElements)
                })
                .map(|rock| (cluster.id.clone(), rock.letter))
        })
        .expect("a strategic rock");
    let now = rig.tw.world().world_ic_now().at;
    let mut tenure = tacsim::surface::claims::BlockTenure::new(cluster.clone(), letter, who);
    tenure.claim = Some(tacsim::surface::claims::BlockClaim {
        holder: PrincipalId::Object(who),
        account: rig
            .tw
            .world()
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.account_of(who)),
        epoch: 1,
        opened_ic: None,
        registered_ic: now,
        term_start_ic: now,
        term_end_ic: now + 365 * 86_400,
        worked_at_term_start: 0,
        discovered_ic: None,
        declared: None,
        fee: None,
        refund: None,
        lapsed_ic: None,
    });
    let handles = rig
        .tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .cloned()
        .expect("the port's handles");
    seed_typed(
        rig,
        &handles.families.claims,
        &tacsim::surface::claims::BlockKey {
            site: cluster,
            letter,
        },
        tenure,
    );
}

/// `who`'s available credit, minor units.
pub(crate) fn credit(rig: &crate::driving_rig::Rig, who: ObjId) -> i64 {
    let account = tacsim::port::credit::own_account(rig.tw.world(), who)
        .expect("an account")
        .id;
    rig.tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().account(account).and_then(|a| a.available()))
        .expect("the account reads")
        .minor()
}

/// The Freeport's record of `who`.
pub(crate) fn freeport_record(
    rig: &crate::driving_rig::Rig,
    who: ObjId,
) -> Option<tacsim::offense::OffenseRecord> {
    let freeport = tacsim::polity::polity(rig.tw.world(), "freeport").expect("the Freeport");
    tacsim::offense::record_of(
        rig.tw.world(),
        &OffenseKey {
            offender: Offender::Character(who),
            polity: freeport,
        },
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lw3_an_audit_finds_strategic_ore_held_without_the_papers() {
    let mut rig = crate::driving_rig::stocked();
    let (holder, row) = prospector(&mut rig, "Holder", 61, "Tern");
    let hull = ObjId(row.hull);
    let hold = hold_vault_of(&rig.tw, &row);
    seed_pool(&mut rig, hold, "strategic_ore", 6, LotId(0x50b1));
    let _ = crate::in5_production::mint(&mut rig, holder);
    let before = credit(&rig, holder);

    let audited = run(&mut rig.tw, GOD, &format!("+law/audit #{}", hull.0));
    assert!(
        audited.contains("held without its papers") && audited.contains("recorded and fined"),
        "{audited}"
    );
    let record = freeport_record(&rig, holder).expect("the Freeport's record");
    assert_eq!(record.rows.len(), 1);
    let row = &record.rows[0];
    assert_eq!(
        (row.kind, &row.proof),
        (OffenseKind::UnlicensedPossession, &Proof::Audit)
    );
    assert!(
        matches!(&row.evidence, Evidence::Audit { units: 6, .. }),
        "{:?}",
        row.evidence
    );
    assert!(row.debt.is_some(), "fined at once");
    assert_eq!(
        before - credit(&rig, holder),
        300_000,
        "6 t at 500 cr: 3,000 cr, paid"
    );

    // Holding the papers, an audit finds nothing.
    seed_strategic_claim(&mut rig, holder);
    let _ = crate::driving_rig::fire(&mut rig);
    let clean = run(&mut rig.tw, GOD, &format!("+law/audit #{}", hull.0));
    assert!(clean.contains("finds nothing restricted"), "{clean}");
    assert_eq!(freeport_record(&rig, holder).map(|r| r.rows.len()), Some(1));
}

/// **A private sale** (doc 50 cut LW.3; shared with doc 50 cut OP.8's third story): `seller`, at
/// Ceres' exchange, lists their hold's lot `lot` of `units` t at 300 cr a tonne to a new trader
/// alone, who quotes, accepts into a fresh locker and settles. `(trader, quote, the accept's reply)`.
pub(crate) fn sell_privately(
    rig: &mut crate::driving_rig::Rig,
    seller: ObjId,
    lot: LotId,
    units: u64,
    (trader_name, seed): (&str, i64),
) -> (ObjId, u64, String) {
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, seller, exchange);
    let trader = pilot(rig, trader_name, seed);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", trader.0),
    );
    let _ = crate::in5_production::mint(rig, trader);
    teleport(&mut rig.tw, trader, exchange);
    let opened = run(&mut rig.tw, trader, "+locker/open");
    assert!(opened.contains("is open"), "{opened}");
    let venue = mush_market::MarketId::new("ceres").expect("a venue");
    let locker = tacsim::port::locker::locker_of(rig.tw.world(), trader, &venue)
        .map(|l| VaultId(l.vault))
        .expect("the locker");

    // The private sale at 300 cr a tonne (the verb's price is minor units), to the trader alone.
    let offered = run(
        &mut rig.tw,
        seller,
        &format!("+market/post {lot} price=30000 to={trader_name}"),
    );
    assert!(offered.contains("Listed for sale as #"), "{offered}");
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    let listing = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.live_listings()
                .find(|l| l.source.exact_lot() == Some(lot))
                .map(|l| l.id.0)
        })
        .expect("the private listing");
    let quoted = run(
        &mut rig.tw,
        trader,
        &format!("+market/quote {listing}={units}"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.quotes_of_buyer(&PrincipalId::Object(trader))
                .map(|q| q.id.0)
                .max()
        })
        .expect("a quote");
    let accepted = run(
        &mut rig.tw,
        trader,
        &format!("+market/accept {quote}={}", locker.0),
    );
    let queued = drain(&mut rig.tw);
    land_all(rig, &queued);
    // The inventory mirror refreshes on its minute sweep; the market's sweep then settles.
    for _ in 0..3 {
        crate::driving_rig::sweep(&mut rig.tw, 60);
        crate::in10_pools::settle(rig, 1);
    }
    (trader, quote, accepted)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn lw3_a_buyer_reports_a_private_sale_hands_over_the_lot_and_is_repaid() {
    let mut rig = crate::driving_rig::stocked();
    let (seller, row) = prospector(&mut rig, "Smuggler", 62, "Tern");
    let hold = hold_vault_of(&rig.tw, &row);
    seed_lot(&mut rig, hold, "strategic_ore", 2, LotId(0x50b2));
    let _ = crate::in5_production::mint(&mut rig, seller);
    let (trader, quote, accepted) =
        sell_privately(&mut rig, seller, LotId(0x50b2), 2, ("Trader", 63));
    let receipt = tacsim::port::law_report::receipts_of(rig.tw.world(), trader)
        .and_then(|b| b.receipts.into_iter().find(|r| r.quote == quote))
        .unwrap_or_else(|| panic!("the trader's receipt ({accepted})"));
    assert_eq!(
        (receipt.seller, receipt.units, receipt.seller_papers),
        (seller, 2, false),
        "the receipt records that the seller lacked the papers"
    );
    assert!(
        tacsim::port::law_report::receipts_of(rig.tw.world(), seller)
            .is_some_and(|b| b.receipts.iter().any(|r| r.quote == quote)),
        "and the seller holds one too"
    );

    // The report: the lot goes to the Registry's vault in the report's own action.
    let seller_before = credit(&rig, seller);
    let trader_before = credit(&rig, trader);
    let filed = run(&mut rig.tw, trader, &format!("+law/report {quote} go"));
    assert!(filed.contains("report is filed"), "{filed}");
    let queued = drain(&mut rig.tw);
    assert_eq!(queued.len(), 1, "one report: {filed}");
    land_all(&mut rig, &queued);
    let registry = tacsim::port::seized::port_authority(rig.tw.world(), &cid(CERES))
        .expect("the Ceres authority");
    let vault = rig
        .tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| m.vendor_of_org(registry.org).map(|v| v.vault))
        .expect("the Registry's vault");
    let families = rig
        .tw
        .world()
        .module_state::<mush_inventory::InventoryHandles>()
        .expect("inventory")
        .0
        .clone();
    assert_eq!(
        crate::driving_rig::typed(&mut rig.tw, &families.lots, &LotId(0x50b2))
            .map(|l| l.endpoint()),
        Some(mush_inventory::EndpointRef(vault)),
        "the trader handed the lot over to the Registry's vault"
    );

    // The tick records the sale against the seller and fines it; the fine is collected at once
    // and the trader is paid the price and a fifth of the fine.
    crate::in10_pools::settle(&mut rig, 3);
    let record = freeport_record(&rig, seller).expect("the seller's record");
    let row = record
        .rows
        .iter()
        .find(|r| r.kind == OffenseKind::RestrictedSale)
        .expect("the sale is proven");
    assert_eq!(row.proof, Proof::Reported { by: trader });
    assert_eq!(
        seller_before - credit(&rig, seller),
        100_000,
        "2 t: the 1,000 cr floor, paid"
    );
    assert!(row.payout.as_ref().is_some_and(|p| p.paid), "{row:?}");
    assert_eq!(
        credit(&rig, trader) - trader_before,
        60_000 + 20_000,
        "the price paid back and a fifth of the fine"
    );
    let again = run(&mut rig.tw, trader, &format!("+law/report {quote} go"));
    assert!(again.contains("already been reported"), "{again}");
}
