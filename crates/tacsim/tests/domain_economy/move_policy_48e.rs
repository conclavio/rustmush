//! **Doc 48e, exchange G7 (48d C-03): a player's `@inv/move` is the ports' to judge.**
//!
//! `@inv/move` is open to players, and D-18-37 left its locality to P-H while a pilot held one
//! vault. EX.1b's lockers ended that: before the fix a player moved goods between any of their
//! vaults from anywhere, past a locker's room, out of a chartered hold and into a listed hull's
//! hold. Now (`PortMovePolicy`, and the room asked inside the prepare):
//!
//! - a move between an owner's docked hold and her locker at the port she stands in lands;
//! - one from outside a port is refused before anything is queued;
//! - one past the locker's room is refused inside the prepare, and nothing moves;
//! - one into a listed hull's hold is refused;
//! - charter cargo stays in the chartered hold.

use mush_core::ObjId;
use mush_economy::VaultId;

use crate::driving_rig::{
    Rig, drain, hold_pool, hold_vault_of, land_quietly, prospector, run, seed_hold, stocked,
    teleport,
};
use crate::hull_resale_ex2::{exchange, list, owner_of_her};

const METALS: &str = "metals";

fn open_locker(rig: &mut Rig, who: ObjId) -> VaultId {
    exchange(rig, who);
    let out = run(&mut rig.tw, who, "+locker/open");
    let venue = mush_market::MarketId::new("ceres").expect("Ceres");
    tacsim::port::locker::locker_of(rig.tw.world(), who, &venue)
        .map(|l| VaultId(l.vault))
        .unwrap_or_else(|| panic!("a locker: {out}"))
}

/// `@inv/move <from>/metals:<n>=<to>` as `who`: what they heard, and whether it queued (landed).
fn move_metals(rig: &mut Rig, who: ObjId, from: VaultId, to: VaultId, n: u64) -> (String, bool) {
    let out = run(
        &mut rig.tw,
        who,
        &format!("@inv/move {}/{METALS}:{n}={}", from.0, to.0),
    );
    let queued = drain(&mut rig.tw);
    let any = !queued.is_empty();
    land_quietly(rig, &queued);
    (out, any)
}

#[test]
fn g7_a_players_move_is_local_bounded_and_keeps_charter_cargo_and_sales() {
    let mut rig = stocked();
    // A chartered pilot's cargo stays aboard.
    let (pilot, charter) = prospector(&mut rig, "Charter", 72, "Brine Lark");
    let charter_hold = hold_vault_of(&rig.tw, &charter);
    seed_hold(&mut rig, &charter, METALS, 5, 4_817);
    let pilot_locker = open_locker(&mut rig, pilot);
    let (out, queued) = move_metals(&mut rig, pilot, charter_hold, pilot_locker, 5);
    assert!(!queued, "{out}");
    assert!(
        out.contains(&tacsim::port::market::MOVE_CHARTER_CARGO.replace("{name}", "Brine Lark")),
        "{out}"
    );
    assert_eq!(hold_pool(&mut rig.tw, charter_hold, METALS), 5);

    let (owner, row) = owner_of_her(&mut rig, "Owner", 71, "Grey Tern");
    let hold = hold_vault_of(&rig.tw, &row);
    seed_hold(&mut rig, &row, METALS, 21, 4_807);
    let locker = open_locker(&mut rig, owner);

    // Where she stands, between her docked hold and her locker: it lands.
    let (out, queued) = move_metals(&mut rig, owner, hold, locker, 5);
    assert!(queued, "{out}");
    assert_eq!(hold_pool(&mut rig.tw, locker, METALS), 5, "{out}");

    // Past the locker's 20 t (5 held, 16 more, all of it in the hold): refused inside the
    // prepare, nothing moved.
    let (out, _) = move_metals(&mut rig, owner, hold, locker, 16);
    assert_eq!(
        hold_pool(&mut rig.tw, locker, METALS),
        5,
        "the locker holds no more than its room: {out}"
    );
    assert_eq!(hold_pool(&mut rig.tw, hold, METALS), 16);

    // From outside any port: refused before anything is queued.
    teleport(&mut rig.tw, owner, mush_core::LIMBO);
    let (out, queued) = move_metals(&mut rig, owner, hold, locker, 1);
    assert!(!queued, "{out}");
    assert!(
        out.contains(tacsim::port::market::MOVE_NOT_AT_VENUE),
        "{out}"
    );

    // 15 more fits (5 + 15 = 20 t); one more tonne does not. Staff take the last tonne off,
    // the hold empties, and she lists her hull: then her hold takes nothing back.
    exchange(&mut rig, owner);
    let (out, queued) = move_metals(&mut rig, owner, hold, locker, 15);
    assert!(queued, "{out}");
    assert_eq!(hold_pool(&mut rig.tw, locker, METALS), 20, "{out}");
    let (out, _) = move_metals(&mut rig, owner, hold, locker, 1);
    assert_eq!(
        hold_pool(&mut rig.tw, locker, METALS),
        20,
        "a full locker: {out}"
    );
    let store = crate::driving_rig::port_map(&rig.tw, "ceres_port").vendors
        [&crate::driving_rig::cid("freeport_exchange")]
        .vault;
    let (out, queued) = move_metals(&mut rig, mush_core::GOD, hold, VaultId(store), 1);
    assert!(queued, "{out}");
    assert_eq!(hold_pool(&mut rig.tw, hold, METALS), 0, "{out}");
    let _listing = list(&mut rig, owner, ObjId(row.hull));
    let (out, queued) = move_metals(&mut rig, owner, locker, hold, 1);
    assert!(!queued, "{out}");
    assert!(
        out.contains(&tacsim::port::market::MOVE_HULL_ON_SALE.replace("{name}", "Grey Tern")),
        "{out}"
    );
}
