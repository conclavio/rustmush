//! **Doc 48e lane L2 — title, hull sale, resale, berth and contracts** (the 48c and 48d reviews'
//! fixes), over the composed lane (`driving`'s rig).
//!
//! - **services:G1 (48d C-06).** A contract offered by someone else, naming a hull its holder
//!   never agreed to sell, does not lock her out of the exchange or the bay.
//! - **services:G7 (48c#13, 48d H-14).** An organization that is no lane institution berths too,
//!   from its own account.
//! - **services:G8 (48d H-13).** A berth's arrears are on `+debts`, and payable there.
//! - **exchange:G17 (48d H-09).** A hull is listed only where she lies docked, and does not undock
//!   while she is for sale.
//! - **production:G7 (48c#10, 48d H-07).** A hold holding a divisible lot is laden: she is not
//!   listed as empty.
//! - **production:G8 (48c#11, 48d H-07).** A hull whose title passes at the berth without an
//!   economy contract (a cored hull's private resale, here a staff transfer standing in for it)
//!   pays her old holder's berth arrears as a sale there does.

use mush_core::{GOD, ObjId};
use mush_economy::{ContractId, ContractState};
use mush_org::PrincipalId;

use crate::driving_rig::{
    board, hold_vault_of, load, port_map, run, seed_lot, set_rate, station_room, stocked, sweep,
    teleport,
};
use crate::hull_resale_ex2::{account_of, balance, list, owner_of_her};

/// The contract number a `@contract/draft` line names.
fn drafted_id(said: &str) -> u64 {
    said.split("Contract #")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.trim_end_matches(':').parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no contract id in {said}"))
}

/// **services:G1**: a griefer drafts a contract with an alt, delivering the victim's hull title
/// from the victim to the alt, and offers it. The victim is no party and never accepted, so it
/// is no sale of hers: she lists her hull and the bay is open to her.
#[test]
fn l2_an_offer_the_holder_never_accepted_does_not_lock_her_hull() {
    let mut rig = stocked();
    let (victim, row) = owner_of_her(&mut rig, "Victim", 71, "Tern Moth");
    let hull = ObjId(row.hull);
    let griefer = rig.tw.spawn_player("Griefer");
    let alt = rig.tw.spawn_player("Alt");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}={}", griefer.0, tacsim::CREDIT),
    );
    let id = drafted_id(&run(&mut rig.tw, griefer, "@contract/draft Lock"));
    let mut heard = Vec::new();
    for line in [
        format!("@contract/party {id}=#{}", alt.0),
        format!(
            "@contract/deliver {id}=title #{} #{}->#{}",
            hull.0, victim.0, alt.0
        ),
        format!("@contract/remainder {id}={}", account_of(&rig, griefer).0),
        format!("@contract/offer {id}"),
    ] {
        let said = run(&mut rig.tw, griefer, &line);
        assert!(!said.contains("Usage"), "{line}: {said}");
        heard.push(format!("{line} -> {said}"));
    }
    let state = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(id)).map(|c| c.state));
    assert_eq!(
        state,
        Some(ContractState::Offered),
        "the griefer's offer stands: {heard:#?}"
    );
    assert!(
        !mush_market::asset_on_sale(rig.tw.world(), hull),
        "an offer she never accepted is no sale of her hull"
    );
    let _ = list(&mut rig, victim, hull);
}

/// **services:G7**: a player's organization, no lane institution, holds a hull's title at Ceres;
/// her berth is charged to it, from its own account. Before, `org_account` read only the port
/// maps' rows, so such a hull berthed free.
#[test]
fn l2_a_player_organization_berths_its_hull() {
    let mut rig = stocked();
    let (founder, row) = owner_of_her(&mut rig, "Founder", 72, "Kite Moth");
    let hull = ObjId(row.hull);
    let mut heard = Vec::new();
    for line in [
        "@org/create Halyard".to_owned(),
        format!("@account/open org:Halyard={}", tacsim::CREDIT),
    ] {
        let out = run(&mut rig.tw, GOD, &line);
        assert!(!out.contains("Permission denied"), "{line}: {out}");
        heard.push(format!("{line} -> {out}"));
    }
    // Her holder gives her title to the organization, as one of its officers (a transfer is the
    // giver's, and an organization receives a title into its root scope by a member it grants).
    let root = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| {
            store
                .state()
                .orgs()
                .find(|org| org.name.as_str() == "Halyard")
                .map(|org| org.root_scope.0)
        })
        .expect("Halyard's root scope");
    let line = format!("@org/grant #{}=Halyard asset.title.transfer", founder.0);
    let out = run(&mut rig.tw, GOD, &line);
    heard.push(format!("{line} -> {out}"));
    let line = format!("@asset/transfer #{}=org:Halyard scope={root}", hull.0);
    let out = run(&mut rig.tw, founder, &line);
    heard.push(format!("{line} -> {out}"));
    let halyard = rig
        .tw
        .world()
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| {
            store
                .state()
                .orgs()
                .find(|org| org.name.as_str() == "Halyard")
                .map(|org| PrincipalId::Organization(org.id))
        })
        .expect("Halyard");
    assert_eq!(
        tacsim::port::house_sweep::title_of(rig.tw.world(), hull),
        Some(halyard.clone()),
        "the organization holds her title: {heard:#?}"
    );
    sweep(&mut rig.tw, 60);
    let ns = tacsim::port::berth::berth_namespace("ceres_port", hull);
    let berth = mush_economy::player_obligations_of(rig.tw.world(), &halyard, &ns)
        .into_iter()
        .find(|o| o.schedule.cadence.is_some());
    assert!(
        berth.is_some(),
        "her berth is the organization's to pay, from its own account"
    );
}

/// **services:G8 (d)**: a broke owner's berth arrears are listed by `+debts`, and paid there.
#[test]
fn l2_berth_arrears_are_on_debts() {
    let mut rig = stocked();
    let (who, row) = owner_of_her(&mut rig, "Debtor", 73, "Wren Moth");
    let hull = ObjId(row.hull);
    set_rate(&mut rig.tw, 24);
    let account = account_of(&rig, who);
    let left = balance(&rig, account);
    let sink = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {}={sink} {left}", account.0),
    );
    assert!(!out.contains("Usage"), "{out}");
    let ns = tacsim::port::berth::berth_namespace("ceres_port", hull);
    for _ in 0..6 {
        sweep(&mut rig.tw, 3_600);
    }
    let owed = tacsim::port::berth::arrears_of(rig.tw.world(), &PrincipalId::Object(who), &ns);
    assert!(owed > 0, "a broke owner's berth runs into arrears");
    let listed = run(&mut rig.tw, who, "+debts");
    assert!(
        listed.contains(tacsim::surface::retrieval::DEBTS_HEAD),
        "the berth's arrears are listed: {listed}"
    );
    let id = mush_economy::debts_of(rig.tw.world(), &PrincipalId::Object(who), &[ns.as_str()])
        .first()
        .map(|o| o.id.0)
        .expect("a berth debt");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {sink}={} {}", account.0, owed + 10_000),
    );
    assert!(!out.contains("Usage"), "{out}");
    let paid = run(&mut rig.tw, who, &format!("+debts/pay {id}"));
    assert!(paid.contains("paid"), "{paid}");
}

/// **exchange:G17**: from Stickney's exchange her owner cannot list a hull that lies at Ceres; at
/// Ceres she lists, and listed she does not undock.
#[test]
fn l2_a_hull_is_listed_where_she_lies_and_stays_while_listed() {
    let mut rig = stocked();
    let (seller, row) = owner_of_her(&mut rig, "Lister", 74, "Pike Moth");
    let hull = ObjId(row.hull);
    let away = station_room(&rig.tw, "stickney_port", "room.p08_exchange");
    teleport(&mut rig.tw, seller, away);
    let far = run(
        &mut rig.tw,
        seller,
        &format!("+market/post #{} price=100000", hull.0),
    );
    assert!(
        far.contains(&tacsim::port::hull_sale::HULL_ELSEWHERE.replace("{name}", "Pike Moth")),
        "not listed from another port: {far}"
    );
    let _ = list(&mut rig, seller, hull);
    load(&mut rig, &row, 60, 20, 0x4c20);
    board(&mut rig, seller);
    let (said, queued) = crate::driving_rig::command(&mut rig, seller, "+ship/undock stickney");
    assert!(
        said.contains(&tacsim::port::voyage::HULL_ON_SALE.replace("{name}", "Pike Moth")),
        "listed, she stays: {said}"
    );
    assert!(queued.is_empty(), "nothing is filed");
}

/// **production:G7**: a divisible lot in her hold (an exact-lot purchase's shape) makes her laden,
/// though no pool of it is: she is not listed as empty.
#[test]
fn l2_a_lot_in_her_hold_makes_her_laden() {
    let mut rig = stocked();
    let (seller, row) = owner_of_her(&mut rig, "Lotter", 75, "Gull Moth");
    let hull = ObjId(row.hull);
    let hold = hold_vault_of(&rig.tw, &row);
    seed_lot(&mut rig, hold, "metals", 3, mush_inventory::LotId(0x4c21));
    crate::hull_resale_ex2::exchange(&mut rig, seller);
    let out = run(
        &mut rig.tw,
        seller,
        &format!("+market/post #{} price=100000", hull.0),
    );
    assert!(
        out.contains(&tacsim::port::hull_sale::HULL_HOLD_LADEN.replace("{name}", "Gull Moth")),
        "a lot is cargo: {out}"
    );
}

/// **production:G8**: a broke owner runs into berth arrears at Ceres; her title passes to another
/// with no economy contract (the shape a cored hull's resale has); once the old holder can pay,
/// the port collects the lien from them.
#[test]
fn l2_a_title_passed_without_a_contract_still_pays_the_berth_lien() {
    let mut rig = stocked();
    let (who, row) = owner_of_her(&mut rig, "Seller", 76, "Rook Moth");
    let hull = ObjId(row.hull);
    let other = rig.tw.spawn_player("Taker");
    set_rate(&mut rig.tw, 24);
    let account = account_of(&rig, who);
    let left = balance(&rig, account);
    let sink = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/pay {}={sink} {left}", account.0),
    );
    assert!(!out.contains("Usage"), "{out}");
    let ns = tacsim::port::berth::berth_namespace("ceres_port", hull);
    let seller = PrincipalId::Object(who);
    for _ in 0..6 {
        sweep(&mut rig.tw, 3_600);
    }
    let owed = tacsim::port::berth::arrears_of(rig.tw.world(), &seller, &ns);
    assert!(owed > 0, "her berth runs into arrears");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", hull.0, other.0),
    );
    assert!(!out.contains("Permission denied"), "{out}");
    assert_eq!(
        tacsim::port::house_sweep::title_of(rig.tw.world(), hull),
        Some(PrincipalId::Object(other)),
        "her title passed at the berth"
    );
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {sink}={} {}", account.0, owed + 10_000),
    );
    assert!(!out.contains("Usage"), "{out}");
    sweep(&mut rig.tw, 30);
    assert_eq!(
        tacsim::port::berth::arrears_of(rig.tw.world(), &seller, &ns),
        0,
        "the lien is collected from her old holder"
    );
}
