//! **Doc 50 unit LW.4 — the standing loss, the trading restriction, the notice and `+record`**
//! (Q-50-39, Q-50-40, Q-50-42, Q-50-69, Q-50-70), over the composed lane (`driving`'s stocked
//! rig), through registered verbs and the port tick.
//!
//! `lw4_an_unpaid_fine_restricts_until_it_is_paid`: a prospector docked at Ceres holds 6 t of
//! strategic ore with a license and no claim, and no money. Staff audit her hull. She is told at
//! once that the Freeport has recorded an offense; the fine (3,000 cr) stands unpaid, so she
//! loses 5 standing and is under the Freeport's restriction. Holding both papers now, the board
//! still refuses her strategic ore and the Registry will not extend her license. `+record` shows
//! the offense, how it was proven, the fine and its grace, and what lifts the restriction;
//! `+debts` lists the fine. Paid, the restriction lifts: the board trades with her again, and
//! the standing she lost stays lost.
//!
//! **What is a fixture.** The ore, seeded; the claim on a strategic rock, a typed record; money.

use mush_core::{GOD, ObjId};
use mush_inventory::LotId;

use crate::driving_rig::{
    cid, command, fire, hold_vault_of, port_map, prospector, run, seed_pool, station_room, teleport,
};
use crate::lw3_law_audit::{credit, freeport_record, seed_strategic_claim};

const CERES: &str = "ceres_port";

/// The Exchange's standing strategic ore bid at Ceres.
fn strategic_bid(rig: &crate::driving_rig::Rig) -> u64 {
    port_map(&rig.tw, CERES).vendors[&cid("freeport_exchange")].bids[&cid("strategic_ore")].listing
}

/// `who`'s standing with the Freeport.
fn standing(rig: &crate::driving_rig::Rig, who: ObjId) -> i16 {
    let freeport = tacsim::polity::polity(rig.tw.world(), "freeport").expect("the Freeport");
    mush_faction::public_standing(rig.tw.world(), freeport, who).map_or(0, |v| v.score)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn lw4_an_unpaid_fine_restricts_until_it_is_paid() {
    let mut rig = crate::driving_rig::stocked();
    let (holder, row) = prospector(&mut rig, "Holder", 71, "Tern");
    let hull = ObjId(row.hull);
    let hold = hold_vault_of(&rig.tw, &row);
    seed_pool(&mut rig, hold, "strategic_ore", 6, LotId(0x50c1));
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, holder, exchange);
    let freeport = tacsim::polity::polity(rig.tw.world(), "freeport").expect("the Freeport");
    // No money: her starting credit goes to the port's treasury.
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), holder)
        .expect("an account")
        .id;
    let available = credit(&rig, holder);
    if available > 0 {
        let out = run(
            &mut rig.tw,
            GOD,
            &format!("@econ/pay #{}=#{treasury} {available}", account.0),
        );
        assert!(out.contains("Paid: transaction #"), "{out}");
    }

    // ---- 1. The audit: the notice at once, the fine unpaid, the standing lost ----
    let events = rig.tw.run(GOD, &format!("+law/audit #{}", hull.0));
    let told = rig.tw.seen_by(&events, holder).join("\n");
    assert!(
        told.contains("Official notice:")
            && told.contains("holding a restricted good without its papers")
            && told.contains("+record"),
        "{told}"
    );
    let record = freeport_record(&rig, holder).expect("the Freeport's record");
    let fine = record.rows[0].debt.expect("fined at once");
    let status = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .obligation(mush_economy::ObligationId(fine))
                .map(|o| o.status.clone())
        });
    assert!(
        !matches!(status, Some(mush_economy::ObligationStatus::Satisfied)),
        "she cannot pay: {status:?}"
    );
    assert_eq!(
        standing(&rig, holder),
        -5,
        "an audit's fine not paid at once"
    );
    assert!(tacsim::port::record::restricted(
        rig.tw.world(),
        holder,
        freeport
    ));

    // ---- 2. Restricted: papers or none, no restricted trade and no license ----
    seed_strategic_claim(&mut rig, holder);
    let _ = fire(&mut rig);
    let now = rig.tw.world().world_ic_now().at;
    assert!(tacsim::port::law::papers_clear(rig.tw.world(), holder, now));
    let bid = strategic_bid(&rig);
    let (refused, queued) = command(&mut rig, holder, &format!("+market/sell {bid}=1"));
    assert!(
        refused.contains("does not let you trade restricted goods"),
        "{refused}"
    );
    assert!(queued.is_empty(), "nothing moves: {refused}");
    let license = run(&mut rig.tw, holder, "+license/buy go");
    assert!(
        license.contains("The Registry issues no new license to your account"),
        "{license}"
    );

    // ---- 3. `+record` and `+debts` ----
    let page = run(&mut rig.tw, holder, "+record");
    assert!(
        page.contains("holding a restricted good without its papers, proven by a port audit")
            && page.contains(&format!("fine #{fine} of"))
            && page.contains("its grace runs to")
            && page.contains("Restrictions in force:")
            && page.contains(&format!("It lifts on paying fine #{fine}.")),
        "{page}"
    );
    let debts = run(&mut rig.tw, holder, "+debts");
    assert!(debts.contains(&format!("+debts/pay {fine}")), "{debts}");

    // ---- 4. Paid: the restriction lifts, the standing stays lost ----
    let _ = crate::in5_production::mint(&mut rig, holder);
    let paid = run(&mut rig.tw, holder, &format!("+debts/pay {fine}"));
    assert!(paid.contains(&format!("Debt #{fine} paid")), "{paid}");
    assert!(!tacsim::port::record::restricted(
        rig.tw.world(),
        holder,
        freeport
    ));
    assert_eq!(
        standing(&rig, holder),
        -5,
        "paying restores nothing (Q-50-42)"
    );
    let page = run(&mut rig.tw, holder, "+record");
    assert!(
        page.contains("No restriction is in force.") && page.contains("paid"),
        "{page}"
    );
    let (sold, queued) = command(&mut rig, holder, &format!("+market/sell {bid}=1"));
    assert!(
        !sold.contains("does not let you trade restricted goods") && !queued.is_empty(),
        "{sold}"
    );
}
