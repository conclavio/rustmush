//! **Doc 48e, exchange G8 (48c #6, 48d C-03, H-07): a divisible good's lots are held.**
//!
//! A captain owns her Dray, whose hold carries 100 t of metals. She crystallizes all of it into
//! one exact lot (`@inv/crystallize`, open to players), so the pool reads 0 and the metals sit in
//! a lot. Before the fix every reader but a part's counted the pool alone, so the full hold read
//! empty:
//!
//! - the hull listed for sale though laden (EX.2's `HULL_HOLD_LADEN`);
//! - a buy into the full hold was admitted (the market's room check);
//!
//! Now the lot is held: the listing is refused, and the buy is refused for room.

use mush_core::ObjId;

use crate::driving_rig::{Rig, drain, hold_pool, hold_vault_of, land_all, run, seed_hold, stocked};
use crate::hull_resale_ex2::{exchange, mint, owner_of_her};
use crate::in5_production::{market, storefront};

const METALS: &str = "metals";

/// `@inv/crystallize <hold>/metals=<n>`, landed.
fn crystallize(rig: &mut Rig, who: ObjId, vault: mush_economy::VaultId, n: u64) {
    let out = run(
        &mut rig.tw,
        who,
        &format!("@inv/crystallize #{}/{METALS}={n}", vault.0),
    );
    let queued = drain(&mut rig.tw);
    assert!(!queued.is_empty(), "{out}");
    land_all(rig, &queued);
}

#[test]
fn g8_a_hold_full_of_a_divisible_lot_is_laden_and_has_no_room() {
    let mut rig = stocked();
    let (captain, row) = owner_of_her(&mut rig, "Holder", 61, "Iron Wren");
    let hull = ObjId(row.hull);
    let vault = hold_vault_of(&rig.tw, &row);
    seed_hold(&mut rig, &row, METALS, 100, 4_808);
    crystallize(&mut rig, captain, vault, 100);
    assert_eq!(
        hold_pool(&mut rig.tw, vault, METALS),
        0,
        "the pool is empty"
    );
    let metals = mush_core::CanonicalId::new(METALS).expect("metals");
    // A laden hull is not listed.
    exchange(&mut rig, captain);
    let out = run(
        &mut rig.tw,
        captain,
        &format!("+market/post #{} price=1000000", hull.0),
    );
    assert!(
        out.contains(&tacsim::port::hull_sale::HULL_HOLD_LADEN.replace("{name}", "Iron Wren")),
        "a hold holding a lot is laden: {out}"
    );
    // A full hold takes no more: the market's room check weighs the lot.
    mint(&mut rig, captain, 10_000_000);
    let ask = {
        let m = market(&rig);
        m.lot_listing_of(
            mush_inventory::EndpointRef(storefront(&rig, "ceres_port", "freeport_exchange")),
            &metals,
        )
        .expect("Freeport Exchange asks metals")
    };
    let quoted = run(&mut rig.tw, captain, &format!("+market/quote {ask}=5"));
    let quote = market(&rig)
        .quotes_of_buyer(&mush_org::PrincipalId::Object(captain))
        .map(|q| q.id)
        .max();
    if let Some(quote) = quote {
        let accepted = run(&mut rig.tw, captain, &format!("+market/accept {quote}"));
        let queued = drain(&mut rig.tw);
        land_all(&mut rig, &queued);
        assert!(
            market(&rig)
                .quote(quote)
                .is_none_or(|q| q.contract.is_none()),
            "a full hold is sold nothing: {accepted}"
        );
    } else {
        assert!(!quoted.contains("Quote #"), "{quoted}");
    }
    assert_eq!(
        hold_pool(&mut rig.tw, vault, METALS),
        0,
        "nothing was drawn into the full hold"
    );
}
