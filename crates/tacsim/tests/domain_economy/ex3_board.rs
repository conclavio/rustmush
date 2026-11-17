//! **Doc 48 cut EX.3 — NPC trade on the board** (Q-48-9; doc 48a A-3), over the composed world.
//!
//! The guarantee each flow states: goods never move without escrowed money, and a missed leg
//! refunds. V6, the Stickney works buying Kasei's alloys: the clerk quotes Kasei's ask and accepts
//! into the works' stores from a purse the operator funds. The first lot is bought while the
//! ports stock up. When the next IC day's lot cannot be drawn (the pool moved away under it), the
//! escrow comes back to the purse and the stores take nothing; with the pool back, a later IC
//! day's purchase settles on the board from the same purse.
//!
//! The gate's agent rule: a port's clerk trades under its presence at its own port's venue only
//! (here); a house helm at the venue its floor buys at only (`in10_pools`, where a helm exists).

use mush_core::{GOD, ObjId};
use mush_economy::VaultId;
use mush_market::VenueGate as _;
use mush_org::PrincipalId;

use crate::driving_rig::{Rig, cid, fire, hold_pool, ic_now, land_quietly, port_map, run, stocked};

const STICKNEY: &str = "stickney_port";
const ALLOYS: &str = "structural_alloys";
const DAY: i64 = 86_400;

fn clerk(rig: &Rig) -> ObjId {
    clerk_of(rig, STICKNEY)
}

fn clerk_of(rig: &Rig, port: &str) -> ObjId {
    port_map(&rig.tw, port)
        .clerk
        .map(|c| ObjId(c.thing))
        .expect("the port's clerk")
}

/// **The venue gate's answer** to `actor` trading at `venue` under its own presence, no
/// endpoint named: `Err` with the refusal's line.
pub(crate) fn admitted(rig: &Rig, actor: ObjId, venue: &str) -> Result<(), String> {
    tacsim::port::PortVenueGate
        .admit(
            rig.tw.world(),
            actor,
            &mush_market::MarketId::new(venue).expect("a venue"),
            None,
            mush_market::Presence::Here,
        )
        .map_err(|refused| refused.line)
}

/// **Doc 48 cut EX.3: a clerk is its own port's agent, and no other's** — Stickney's clerk and
/// Ceres's, standing in their staff spines, each trade at their own port's venue under their
/// presence and are refused at the other's, in the gate's own words.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex3_a_clerk_trades_at_its_own_port_only() {
    let rig = stocked();
    let (stickney, ceres) = (clerk_of(&rig, STICKNEY), clerk_of(&rig, "ceres_port"));
    assert_eq!(admitted(&rig, stickney, "stickney"), Ok(()));
    assert_eq!(admitted(&rig, ceres, "ceres"), Ok(()));
    for (clerk, elsewhere) in [(stickney, "ceres"), (ceres, "stickney")] {
        let refused = admitted(&rig, clerk, elsewhere).expect_err("refused at another port");
        assert!(refused.starts_with("That market is "), "{refused}");
    }
}

/// **Doc 48 cut EX.3: the works' stores stand in the operator's root scope** (they had none) —
/// Stickney's clerk may take from them through its `vault.withdraw` grant there, and a player who
/// is not the operator's agent may not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex3_the_stores_admit_the_clerk_and_no_player() {
    let mut rig = stocked();
    let stores = mush_inventory::EndpointRef(
        crate::in5_production::stores(&rig, STICKNEY, "room.p15_maintenance")
            .expect("the works' stores"),
    );
    let player = crate::driving_rig::pilot(&mut rig, "Okafor", 11);
    let w = rig.tw.world();
    assert!(
        mush_inventory::may_take(w, clerk(&rig), stores),
        "the clerk"
    );
    assert!(!mush_inventory::may_take(w, player, stores), "a player");
}

fn vendor_vault(rig: &Rig, org: &str) -> VaultId {
    VaultId(port_map(&rig.tw, STICKNEY).vendors[&cid(org)].vault)
}

/// The clerk's purchases (the works' only buyer): `(one on the board, settled trades)`. Counted
/// over every listing, since the stock arm relists Kasei's ask under a new id.
fn purchases(rig: &Rig) -> (bool, usize) {
    let (w, clerk) = (rig.tw.world(), clerk(rig));
    let buyer = PrincipalId::Object(clerk);
    w.module_state::<mush_market::MarketStore>()
        .map_or((false, 0), |m| {
            (
                m.quotes_of_buyer(&buyer)
                    .any(|q| tacsim::port::supply::on_board(w, clerk, q.listing)),
                m.quotes_of_buyer(&buyer).filter(|q| q.is_trade()).count(),
            )
        })
}

/// Every quote in the market, `listing buyer contract settled`, for a failure's message.
fn board(rig: &Rig) -> String {
    let clerk = clerk(rig);
    rig.tw
        .world()
        .module_state::<mush_market::MarketStore>()
        .map(|m| {
            m.quotes()
                .map(|q| {
                    format!(
                        "#{} on {} by {:?}{} c={:?} settled={}",
                        q.id.0,
                        q.listing.0,
                        q.buyer,
                        if q.buyer == PrincipalId::Object(clerk) {
                            " (the clerk)"
                        } else {
                            ""
                        },
                        q.contract,
                        q.is_trade()
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default()
}

/// The clerk's purse: its own available credit, minor units.
fn purse(rig: &Rig) -> i64 {
    tacsim::port::credit::own_account(rig.tw.world(), clerk(rig))
        .and_then(mush_economy::Account::available)
        .map_or(0, mush_economy::Amount::minor)
}

/// The world clock moved, without a fire, to a minute past the next IC-day boundary.
fn to_next_day(rig: &mut Rig) {
    let rate = i64::from(crate::driving_rig::world_rate(&rig.tw).max(1));
    let now = ic_now(&rig.tw);
    let ahead = (now.div_euclid(DAY) + 1) * DAY + 60 - now;
    let t = rig
        .tw
        .world()
        .now()
        .saturating_add((ahead + rate - 1) / rate);
    rig.tw.set_now(t);
}

/// The engine's queue filled by strangers' lot actions (doc 18 cut IC14 row 2's rig).
fn fill_queue(rig: &mut Rig) {
    let action = mush_market::lot_action_ref(&rig.tw.engine().registry).expect("the lot action");
    for i in 0..64_u32 {
        rig.tw
            .engine_mut()
            .queue_authoritative_submit(Box::new(mush_core::AuthoritativeSubmit {
                actor: ObjId(900_000 + i),
                action: action.clone(),
                subject: mush_core::PerceivedRef::SelfActor,
                targets: Vec::new(),
                parameters: serde_json::json!({}),
                key: mush_core::IdempotencyKeyText::parse_canonical("ex3.filler").expect("a key"),
            }))
            .expect("the queue has room");
    }
}

/// Every tonne of Kasei's alloys moved by staff hand from `from` to `to`, the moves landed.
fn move_alloys(rig: &mut Rig, from: VaultId, to: VaultId) {
    let pool = hold_pool(&mut rig.tw, from, ALLOYS);
    let moved = run(
        &mut rig.tw,
        GOD,
        &format!("@inv/move {}/{ALLOYS}:{pool}={}", from.0, to.0),
    );
    assert!(!moved.contains("Usage"), "{moved}");
    let staff = crate::driving_rig::drain(&mut rig.tw);
    crate::driving_rig::land_all(rig, &staff);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex3_the_works_buys_on_the_board_and_a_missed_leg_refunds() {
    let mut rig = stocked();
    let stores = crate::in5_production::stores(&rig, STICKNEY, "room.p15_maintenance")
        .expect("the works' stores");
    let (kasei, yards) = (
        vendor_vault(&rig, "kasei"),
        vendor_vault(&rig, "stickney_yards"),
    );
    // The stores start under their floor, so the first lot is bought while the ports stock up.
    let (_, first) = purchases(&rig);
    let held = hold_pool(&mut rig.tw, stores, ALLOYS);
    assert!(
        first >= 1 && held >= 5,
        "the first lot bought on the board: {first} trades, the stores at {held}; the board: {}",
        board(&rig)
    );

    // The next IC day's purchase goes on the board; before its draw lands, every tonne of
    // Kasei's alloys moves away by staff hand, so the lot's leg cannot be staged.
    to_next_day(&mut rig);
    // While the engine's queue is full the purchase waits, its IC day unspent: a draw staged
    // into a full queue would be dropped after the escrow.
    for _ in 0..12 {
        fill_queue(&mut rig);
        let (_, queued) = fire(&mut rig);
        assert!(
            !purchases(&rig).0,
            "nothing bought into a full queue: {}",
            board(&rig)
        );
        let theirs: Vec<_> = queued.into_iter().filter(|s| s.actor.0 < 900_000).collect();
        land_quietly(&mut rig, &theirs);
    }
    let mut missed = false;
    for _ in 0..40 {
        let (_, queued) = fire(&mut rig);
        if purchases(&rig).0 {
            move_alloys(&mut rig, kasei, yards);
            land_quietly(&mut rig, &queued);
            missed = true;
            break;
        }
        land_quietly(&mut rig, &queued);
    }
    assert!(
        missed,
        "the clerk put a purchase on the board (the stores at {}; the board: {})",
        hold_pool(&mut rig.tw, stores, ALLOYS),
        board(&rig)
    );
    let escrowed = purse(&rig);

    // Past the contract's deadline: the missed leg refunds, and nothing reached the stores.
    crate::in10_pools::days_on(&mut rig, 1);
    let (on_board, trades) = purchases(&rig);
    assert!(!on_board, "the missed purchase is closed: {}", board(&rig));
    assert_eq!(trades, first, "no trade without its goods: {}", board(&rig));
    assert_eq!(
        hold_pool(&mut rig.tw, stores, ALLOYS),
        held,
        "the stores took nothing"
    );
    assert!(
        purse(&rig) > escrowed,
        "the escrow came back to the purse: {escrowed} then {}",
        purse(&rig)
    );
    let refunded = purse(&rig);

    // Kasei's stock back, a later IC day's purchase settles on the board.
    move_alloys(&mut rig, yards, kasei);
    crate::in10_pools::days_on(&mut rig, 2);
    for _ in 0..12 {
        if purchases(&rig).1 > first {
            break;
        }
        crate::in10_pools::settle(&mut rig, 1);
    }
    assert!(
        purchases(&rig).1 > first,
        "a purchase settled on the board: {}",
        board(&rig)
    );
    assert!(
        hold_pool(&mut rig.tw, stores, ALLOYS) > held,
        "the stores took the lot"
    );
    // The operator topped up only what the purse lacked: the lot was paid from the refund.
    assert!(
        purse(&rig) < refunded,
        "the lot paid from the purse: {refunded} then {}",
        purse(&rig)
    );
}
