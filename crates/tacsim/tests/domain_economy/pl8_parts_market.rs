//! **Doc 48 unit PL.8 — the parts market** (Q-48-29 as doc 48b G-9 corrects it), over the
//! composed world (`driving`'s keyed rig), through the production command paths.
//!
//! Lane v41 gives both workshops' operators (Occator at Ceres, Stickney Yards at Stickney) a
//! standing bid for every part a bay fits, at its reference × 600 ‰. A port house's bid takes no
//! worn part (the gate's `house_takes`), a hot part is held under its claim, and a fitted one is
//! consumed. The clerk lists the sound parts the workshop took in as lot asks at the reference.
//!
//! 1. **The bids.** Each of the thirteen bay-fittable parts has a bid at each workshop, at its
//!    reference × 600 ‰, for four.
//! 2. **An intact part sells.** A captain at Ceres sells an intact 105 mm cannon from her docked
//!    hold into Occator's bid: the lot itself moves into Occator's storefront, and the bid wants
//!    three more.
//! 3. **A damaged part does not.** Her worn coaxial mount is refused in the market's words
//!    ([`mush_market::NOT_AS_HELD`]) and stays aboard: a damaged part is scrap.
//! 4. **The workshop lists what it took in.** The clerk lists the cannon as a public lot ask at
//!    its reference, and a second captain buys it: the same lot into her hold.
//! 5. **The listing's rule.** Of three parts in Occator's storefront (stand-ins for what the
//!    buy-back desk bought from scraps), an intact laser and a made sight (minted new, at the
//!    works' grade) are listed and a worn thermal sight is not.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the parts in the hold and the storefront, at the grades a strip
//! mints their conditions at; the money. Every claim is reached by the players' commands and the
//! port tick. The hot part's hold is `in2_lifecycle`'s (a stripped, claimed part).

use std::collections::BTreeSet;

use mush_core::{CanonicalId, GOD, ObjId};
use mush_economy::VaultId;
use mush_inventory::LotId;
use mush_inventory::record::EndpointRef;
use mush_market::{ListingId, MarketStore, MarketVisibility, Supply};
use mush_tactics::field_ops::{ComponentCondition, ProvenanceLegalityStatus};
use tacsim::port::bay_work::{INTACT_GRADE, WORN_GRADE};

use crate::driving_rig::{
    Rig, at_counter, cid, drain, hold_vault_of, land_all, land_quietly, port_map, prospector, run,
    seed_part_at, stocked, store, sweep, typed,
};

const CERES: &str = "ceres_port";
const STICKNEY: &str = "stickney_port";
const CANNON: &str = "tacsim.part.weapon.cannon_105";
const MOUNT: &str = "tacsim.part.weapon.mg_coax";
const LASER: &str = "tacsim.part.weapon.laser_mk1";
const THERMAL: &str = "tacsim.part.sensor.thermal_mk1";
const SIGHT: &str = "tacsim.part.sensor.visual_mk1";
/// The grade the works mints a made part at (the industry kernel's output grade).
const MADE_GRADE: u8 = 0;

fn mint(rig: &mut Rig, pilot: ObjId) {
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let account = tacsim::port::credit::own_account(rig.tw.world(), pilot)
        .map(|a| a.id.0)
        .expect("the pilot's account");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={account} 10000000"),
    );
}

fn market(rig: &Rig) -> &MarketStore {
    rig.tw
        .world()
        .module_state::<MarketStore>()
        .expect("the market")
}

fn lot_at(rig: &mut Rig, lot: LotId) -> Option<EndpointRef> {
    let inventory = rig
        .tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .and_then(|h| h.inventory.clone())
        .expect("inventory");
    typed(&mut rig.tw, &inventory.lots, &lot).map(|l| l.endpoint())
}

/// Port fires (31 s apart), every submission they stage landed.
fn fires(rig: &mut Rig, n: usize) {
    for _ in 0..n {
        sweep(&mut rig.tw, 31);
        let queued = drain(&mut rig.tw);
        land_quietly(rig, &queued);
    }
}

/// The lot ask that stands on `lot`, if one does.
fn lot_ask(rig: &Rig, lot: LotId) -> Option<mush_market::Listing> {
    market(rig)
        .lot_listings()
        .find(|l| l.source.exact_lot() == Some(lot))
        .cloned()
}

/// A part's reference, minor units.
fn reference_minor(rig: &Rig, good: &str) -> i64 {
    let goods = store(&rig.tw).goods().expect("the goods table");
    let major = goods
        .good(&cid(good))
        .map(|g| g.reference_major)
        .unwrap_or_else(|| panic!("{good} is a good"));
    i64::try_from(major * 100).expect("fits")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn pl8_a_workshop_buys_sound_parts_and_lists_what_it_took_in() {
    let mut rig = stocked();

    // ---- (1) The bids: every bay-fittable part, both workshops, reference × 600 ‰ ----
    let parts: BTreeSet<CanonicalId> = store(&rig.tw)
        .definitions()
        .parts
        .values()
        .map(|part| part.commodity.clone())
        .collect();
    assert_eq!(parts.len(), 13, "the thirteen parts a bay fits");
    for (entry, org) in [(CERES, "occator"), (STICKNEY, "stickney_yards")] {
        let row = port_map(&rig.tw, entry).vendors[&cid(org)].clone();
        for good in &parts {
            let bid = row
                .bids
                .get(good)
                .unwrap_or_else(|| panic!("{org} bids for {good}"));
            let listing = market(&rig)
                .listing(ListingId(bid.listing))
                .cloned()
                .unwrap_or_else(|| panic!("{org}'s {good} bid stands"));
            assert!(
                matches!(listing.source, Supply::Wanted { .. }),
                "{org}'s {good}: a bid"
            );
            assert_eq!(
                listing.price.minor(),
                reference_minor(&rig, good.as_str()) * 600 / 1_000,
                "{org}'s {good}: the reference × 600 permille"
            );
            assert_eq!(listing.quantity, 4, "{org}'s {good}: four wanted");
        }
    }

    // ---- (2) An intact cannon sells into Occator's bid ----
    let (seller, seller_row) = prospector(&mut rig, "Seller", 1, "Salt Moth");
    mint(&mut rig, seller);
    at_counter(&mut rig, seller);
    let hold = hold_vault_of(&rig.tw, &seller_row);
    let (cannon, worn) = (LotId(9_810), LotId(9_811));
    seed_part_at(
        &mut rig,
        hold,
        CANNON,
        cannon,
        (ComponentCondition::Intact, INTACT_GRADE),
        ProvenanceLegalityStatus::CleanTitle,
    );
    seed_part_at(
        &mut rig,
        hold,
        MOUNT,
        worn,
        (ComponentCondition::Damaged, WORN_GRADE),
        ProvenanceLegalityStatus::CleanTitle,
    );
    let occator = port_map(&rig.tw, CERES).vendors[&cid("occator")].clone();
    let storefront = EndpointRef(VaultId(occator.vault));
    let cannon_bid = occator.bids[&cid(CANNON)].listing;
    let sold = run(&mut rig.tw, seller, &format!("+market/sell {cannon_bid}=1"));
    assert!(sold.contains("Sold "), "{sold}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    fires(&mut rig, 12);
    assert_eq!(
        lot_at(&mut rig, cannon),
        Some(storefront),
        "the lot itself is Occator's now"
    );
    assert_eq!(
        market(&rig)
            .listing(ListingId(cannon_bid))
            .map(|l| l.quantity),
        Some(3),
        "the bid wants three more"
    );

    // ---- (3) The worn mount is refused, and stays aboard ----
    let mount_bid = occator.bids[&cid(MOUNT)].listing;
    let refused = run(&mut rig.tw, seller, &format!("+market/sell {mount_bid}=1"));
    assert!(refused.contains(mush_market::NOT_AS_HELD), "{refused}");
    assert!(drain(&mut rig.tw).is_empty(), "nothing is staged");
    assert_eq!(lot_at(&mut rig, worn), Some(EndpointRef(hold)));

    // ---- (4) The clerk lists the cannon; a second captain buys it ----
    fires(&mut rig, 12);
    let ask = lot_ask(&rig, cannon).expect("the workshop lists the cannon");
    assert_eq!(
        ask.price.minor(),
        reference_minor(&rig, CANNON),
        "at its reference"
    );
    assert_eq!(ask.visibility, MarketVisibility::Public);
    assert_eq!(Some(ask.vendor.0), occator.vendor, "Occator's ask");
    let (buyer, buyer_row) = prospector(&mut rig, "Buyer", 2, "Red Moth");
    mint(&mut rig, buyer);
    at_counter(&mut rig, buyer);
    let quoted = run(&mut rig.tw, buyer, &format!("+market/quote {}", ask.id));
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = market(&rig)
        .quotes_of_buyer(&mush_org::PrincipalId::Object(buyer))
        .map(|q| q.id)
        .max()
        .expect("a quote");
    let accepted = run(&mut rig.tw, buyer, &format!("+market/accept {quote}"));
    let queued = drain(&mut rig.tw);
    assert!(!queued.is_empty(), "the hand-over is staged: {accepted}");
    land_all(&mut rig, &queued);
    fires(&mut rig, 6);
    assert_eq!(
        lot_at(&mut rig, cannon),
        Some(EndpointRef(hold_vault_of(&rig.tw, &buyer_row))),
        "the same lot, in the buyer's hold"
    );

    // ---- (5) The listing's rule: sound parts listed, a worn one not ----
    let (laser, made, thermal) = (LotId(9_812), LotId(9_813), LotId(9_814));
    let shop = VaultId(occator.vault);
    for (lot, good, condition, grade) in [
        (laser, LASER, ComponentCondition::Intact, INTACT_GRADE),
        (made, SIGHT, ComponentCondition::Intact, MADE_GRADE),
        (thermal, THERMAL, ComponentCondition::Damaged, WORN_GRADE),
    ] {
        seed_part_at(
            &mut rig,
            shop,
            good,
            lot,
            (condition, grade),
            ProvenanceLegalityStatus::CleanTitle,
        );
    }
    fires(&mut rig, 24);
    let laser_ask = lot_ask(&rig, laser).expect("the intact laser is listed");
    assert_eq!(laser_ask.price.minor(), reference_minor(&rig, LASER));
    let made_ask = lot_ask(&rig, made).expect("the made sight is listed");
    assert_eq!(made_ask.price.minor(), reference_minor(&rig, SIGHT));
    assert!(lot_ask(&rig, thermal).is_none(), "a worn part is scrap");
    assert_eq!(lot_at(&mut rig, thermal), Some(EndpointRef(shop)));
}
