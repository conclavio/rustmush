//! **Doc 50 unit LW.1 — the law asked for a venue and a good** (Q-50-32, Q-50-33, Q-50-34,
//! Q-50-65), over the composed lane (`driving`'s stocked rig), through the market's own verbs.
//!
//! Strategic ore is restricted at Ceres: the Freeport, which holds Ceres de facto, lifts it only
//! for a holder of the picket's papers, a prospecting license **and** a registered claim on a
//! strategic rock (lane v47's terms row, paper `freeport_prospecting`).
//!
//! 1. **The seller into a house bid.** A licensed prospector holding no claim is refused at
//!    `+market/sell` into the Exchange's strategic ore bid: a license alone does not lift it.
//!    With a live claim on a strategic rock as well, the sale goes through.
//! 2. **The buyer.** The prospector posts a public ask of a lot. A trader with no license is
//!    refused a quote on it, and still refused holding a claim: the claim alone does not lift it
//!    either.
//! 3. **A private offer goes through** (Q-50-34): the same prospector offers a lot to the trader
//!    alone, and the trader's quote is taken.
//! 4. **The law follows control** (Q-50-33): staff hand Ceres station to the Republic de facto,
//!    whose terms restrict nothing at Ceres, and the trader may quote the public ask.
//!
//! **What is a fixture.** The ore in the hold, seeded; the claim on a strategic rock, seeded as a
//! typed record (staking one means flying to the rock, which is not this unit's); the money.

use mush_core::{GOD, ObjId};
use mush_inventory::LotId;
use mush_org::PrincipalId;

use crate::driving_rig::{
    cid, command, drain, fire, hold_vault_of, land_all, pilot, port_map, prospector, run, seed_lot,
    seed_pool, seed_typed, station_room, teleport,
};

const CERES: &str = "ceres_port";

/// The refusal's words, whatever the venue and the good are called.
const REFUSED: &str = "only with a holder of a Freeport Registry prospecting license and a \
     registered claim on a strategic rock";

/// The `n`th strategic rock of the lane's orbital clusters: its cluster and letter.
fn strategic_rock(rig: &crate::driving_rig::Rig, n: usize) -> (mush_core::CanonicalId, char) {
    tacsim::space::mining::all_orbital_clusters(rig.tw.world())
        .into_iter()
        .flat_map(|cluster| {
            cluster
                .rocks
                .iter()
                .filter(|rock| {
                    rock.spectral
                        .families()
                        .contains(&tacsim::surface::OreFamily::StrategicElements)
                })
                .map(|rock| (cluster.id.clone(), rock.letter))
                .collect::<Vec<_>>()
        })
        .nth(n)
        .expect("enough strategic rocks")
}

/// A live registered claim on the `rock`th strategic rock, held by `who` (a typed record, by
/// hand).
fn seed_strategic_claim(rig: &mut crate::driving_rig::Rig, who: ObjId, rock: usize) {
    let (cluster, letter) = strategic_rock(rig, rock);
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

/// The Exchange's standing strategic ore bid at Ceres.
fn strategic_bid(rig: &crate::driving_rig::Rig) -> u64 {
    port_map(&rig.tw, CERES).vendors[&cid("freeport_exchange")].bids[&cid("strategic_ore")].listing
}

/// The newest quote `who` holds.
fn newest_quote(rig: &crate::driving_rig::Rig, who: ObjId) -> Option<u64> {
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()?
        .quotes_of_buyer(&PrincipalId::Object(who))
        .map(|q| q.id.0)
        .max()
}

/// The listing `lot` is posted under.
fn listing_of(rig: &crate::driving_rig::Rig, lot: LotId) -> u64 {
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .and_then(|m| {
            m.live_listings()
                .find(|l| l.source.exact_lot() == Some(lot))
                .map(|l| l.id.0)
        })
        .expect("the lot's listing")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn lw1_the_ceres_board_trades_strategic_ore_only_with_the_picket_papers() {
    let mut rig = crate::driving_rig::stocked();
    let (seller, row) = prospector(&mut rig, "Prospector", 51, "Tern");
    let hold = hold_vault_of(&rig.tw, &row);
    seed_pool(&mut rig, hold, "strategic_ore", 6, LotId(0x50a1));
    seed_lot(&mut rig, hold, "strategic_ore", 1, LotId(0x50a2));
    seed_lot(&mut rig, hold, "strategic_ore", 1, LotId(0x50a3));
    let _ = crate::in5_production::mint(&mut rig, seller);
    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, seller, exchange);
    let bid = strategic_bid(&rig);

    // ---- 1. The seller into a house bid: both papers, or nothing ----
    let now = rig.tw.world().world_ic_now().at;
    assert!(
        tacsim::port::licence::licence_gate(rig.tw.world(), seller, now).is_ok(),
        "a prospector holds the Registry's license"
    );
    let (refused, queued) = command(&mut rig, seller, &format!("+market/sell {bid}=1"));
    assert!(
        refused.contains(REFUSED),
        "licensed, with no claim: {refused}"
    );
    assert!(queued.is_empty(), "nothing moves: {refused}");
    seed_strategic_claim(&mut rig, seller, 0);
    let _ = fire(&mut rig);
    let (sold, queued) = command(&mut rig, seller, &format!("+market/sell {bid}=1"));
    assert!(
        !sold.contains(REFUSED) && !queued.is_empty(),
        "with both papers the sale goes through: {sold}"
    );
    land_all(&mut rig, &queued);

    // ---- 2. The buyer on the board: refused without the papers ----
    let listed = run(
        &mut rig.tw,
        seller,
        &format!("+market/post {} price=700", LotId(0x50a2)),
    );
    assert!(listed.contains("Listed for sale as #"), "{listed}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let public = listing_of(&rig, LotId(0x50a2));
    let trader = pilot(&mut rig, "Trader", 52);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", trader.0),
    );
    teleport(&mut rig.tw, trader, exchange);
    let quoted = run(&mut rig.tw, trader, &format!("+market/quote {public}=1"));
    assert!(quoted.contains(REFUSED), "an unlicensed buyer: {quoted}");
    assert_eq!(newest_quote(&rig, trader), None, "no quote is written");
    seed_strategic_claim(&mut rig, trader, 1);
    let _ = fire(&mut rig);
    let quoted = run(&mut rig.tw, trader, &format!("+market/quote {public}=1"));
    assert!(
        quoted.contains(REFUSED),
        "a claim with no license: {quoted}"
    );

    // ---- 3. A private offer goes through ----
    let offered = run(
        &mut rig.tw,
        seller,
        &format!("+market/post {} price=650 to=Trader", LotId(0x50a3)),
    );
    assert!(offered.contains("Listed for sale as #"), "{offered}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let private = listing_of(&rig, LotId(0x50a3));
    let quoted = run(&mut rig.tw, trader, &format!("+market/quote {private}=1"));
    assert!(
        quoted.contains("Quote #"),
        "a private offer is not asked the law: {quoted}"
    );

    // ---- 4. The law follows control ----
    let flipped = run(
        &mut rig.tw,
        GOD,
        "@faction/control Ceres station=Republic/defacto",
    );
    assert!(flipped.contains("holds Ceres station"), "{flipped}");
    let quoted = run(&mut rig.tw, trader, &format!("+market/quote {public}=1"));
    assert!(
        quoted.contains("Quote #"),
        "the Republic restricts nothing at Ceres: {quoted}"
    );
}
