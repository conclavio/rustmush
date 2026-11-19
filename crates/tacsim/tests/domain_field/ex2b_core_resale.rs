//! **Doc 48 unit EX.2b: a governed core resold to a player** (Q-48-47; Q-48-3's correction; the
//! core leg of Q-48-52), by the players' own registered verbs, on the launch world.
//!
//! A governed core's title is a managed title: it changes hands only by its seller's private
//! offer to one named player (`+core/offer`), and the buyer's acceptance (`+core/accept`) is one
//! union in which the buyer's money is taken and split (the seller's net, the venue's cut, the
//! Registry's levy), the title passes and the core moves into the buyer's hands, or nothing does.
//! A ship with a governed core fitted sells the same way, her core's title and hers and her hold
//! in the one union.
//!
//! What staff do: top up wallets from the world's faucet and open the buyers' credit accounts
//! (the M8 world's standing practice), and, in the second story, put a lien on the ship and
//! release it (a claim the sale must respect). Nothing else is staff's.

use mush_core::ObjId;
use mush_industry::core_production::DieClass;
use mush_org::PrincipalId;
use serde_json::json;
use tacsim::hardware::sale::resale as r;
use tacsim::launch::end_user::END_USER_DONE;

use crate::excursions::{economy, god, run};
use crate::m8_end_user::{credit, fund};
use crate::m8_world::{M8World, m8_world_field, title_of};

/// The head of a template, before its first `{`.
fn head(template: &str) -> &str {
    template.split('{').next().expect("a head")
}

/// Where `thing` is.
fn place_of(world: &M8World, thing: ObjId) -> Option<ObjId> {
    world.rig.tw.world().get(thing).and_then(|o| o.location)
}

/// An account's posted balance.
fn balance(world: &M8World, account: mush_economy::AccountId) -> i64 {
    economy(&world.rig.tw)
        .snapshot()
        .account(account)
        .expect("the account")
        .posted_balance
        .minor()
}

/// A second character, with a gateway account and a credit account of their own.
fn player(world: &mut M8World, name: &str, seed: i64) -> ObjId {
    let who = world.rig.tw.spawn_player(name);
    crate::excursions::with_account(&mut world.rig, who, name, seed);
    god(
        &mut world.rig,
        &format!("@account/open #{}=tacsim.credit", who.0),
    );
    who
}

/// `who` types `line` and every submission it queues lands; what was said, and each landing's
/// receipt or refusal.
fn says(
    world: &mut M8World,
    who: ObjId,
    line: &str,
) -> (String, Vec<Result<Option<String>, String>>) {
    let (out, queued) = crate::excursions::command(&mut world.rig, who, line);
    let landed = queued
        .iter()
        .map(|submit| crate::excursions::land_receipt(&mut world.rig, submit))
        .collect();
    (out, landed)
}

/// The one receipt a line's one landing gave.
fn landed_one(line: &str, landed: &[Result<Option<String>, String>]) -> String {
    match landed {
        [Ok(Some(receipt))] => receipt.clone(),
        other => panic!("{line}: one landing with a receipt, not {other:?}"),
    }
}

/// The resale's split at the provisional rates: (venue, levy, seller).
fn split(price: i64) -> (i64, i64, i64) {
    use mush_economy::registry_filing::core_sale::resale::{LEVY_PERMILLE, VENUE_PERMILLE};
    let venue = price * i64::from(VENUE_PERMILLE) / 1_000;
    let levy = price * i64::from(LEVY_PERMILLE) / 1_000;
    (venue, levy, price - venue - levy)
}

/// **A stored core is resold to the one player its seller names**, by their own verbs:
///
/// 1. the pilot offers the core in their hands to a buyer at a price: one offer record, and
///    nothing moves or is held;
/// 2. a third party is refused, by the verb and by the action itself (the provider reads the
///    offer's named buyer), nothing paid or moved;
/// 3. the buyer reads the offer and accepts it: in one union the buyer's account pays the price
///    (the pilot their net, the venue its cut, the Registry its levy), the core's title is the
///    buyer's and the core is in the buyer's hands; one economy receipt and one kernel record,
///    the offer gone, and the fitting shop's buy-back family untouched;
/// 4. a restart replays the resale and pairs the records; the pilot can no longer offer the
///    core, and the buyer can.
#[test]
fn ex2b_a_stored_core_is_resold_to_the_player_its_seller_names() {
    let mut world = m8_world_field(("ahuna", "ahuna_port"), 2);
    let pilot = world.pilot;
    fund(&mut world, pilot, 9_000_000);
    let (payer, _) = credit(&world, pilot);
    let done = run(&mut world.rig.tw, pilot, "+enduser/apply go");
    assert!(done.contains(head(END_USER_DONE)), "{done}");
    let (_, core) = world.integrate(pilot, payer, DieClass::I);
    assert_eq!(place_of(&world, core), Some(pilot), "in the pilot's hands");
    let buyer = player(&mut world, "Corvina", 9_101);
    let third = player(&mut world, "Tertia", 9_102);
    fund(&mut world, buyer, 1_000_000);
    fund(&mut world, third, 1_000_000);
    let venue = world.deployment.venue.recipient.account;
    let levy = world.deployment.registry.account;
    let price = 500_000;
    let (cut, tax, net) = split(price);
    let accounts = |world: &M8World| {
        [
            credit(world, pilot).1,
            credit(world, buyer).1,
            credit(world, third).1,
            balance(world, venue),
            balance(world, levy),
        ]
    };

    // 1. The offer: one record, nothing moved.
    let before = accounts(&world);
    let title = title_of(&world, core);
    let line = format!("+core/offer #{}=#{} price={price}", core.0, buyer.0);
    let (out, landed) = says(&mut world, pilot, &line);
    let receipt = landed_one(&line, &landed);
    assert!(
        receipt.contains(head(r::RESALE_OFFERED)),
        "{out} / {receipt}"
    );
    assert_eq!(world.rows(r::OFFER_FAMILY).len(), 1);
    assert_eq!(accounts(&world), before, "an offer pays and holds nothing");
    assert_eq!(title_of(&world, core), title, "nor moves the title");
    assert_eq!(place_of(&world, core), Some(pilot));
    let listed = run(&mut world.rig.tw, buyer, "+core/offers");
    assert!(listed.contains(&format!("Core #{}", core.0)), "{listed}");

    // 2. A third party: the verb answers there is no offer for them, and the action itself,
    // typed straight to the provider, is refused. Nothing paid or moved.
    for verb in [
        format!("+core/accept #{}", core.0),
        format!("+core/accept #{}=go", core.0),
    ] {
        let (out, landed) = says(&mut world, third, &verb);
        assert!(out.contains(r::RESALE_NO_OFFER), "{verb}: {out}");
        assert!(landed.is_empty(), "{verb}: nothing submitted");
    }
    let refused = world.try_perform("tacsim", r::RESALE_ACTION, third, json!({"core": core.0}));
    assert!(
        refused.is_err(),
        "the provider refuses a third party: {refused:?}"
    );
    assert_eq!(accounts(&world), before);
    assert_eq!(place_of(&world, core), Some(pilot));
    assert_eq!(title_of(&world, core), title);

    // Doc 48e (48d H-07): a stored core changes hands where buyer and seller both stand.
    let away = crate::driving_rig::station_room(&world.rig.tw, "ceres_port", "room.c07_exchange");
    crate::driving_rig::teleport(&mut world.rig.tw, buyer, away);
    let (out, landed) = says(&mut world, buyer, &format!("+core/accept #{}=go", core.0));
    assert!(
        out.contains(r::RESALE_APART),
        "apart, nothing changes hands: {out}"
    );
    assert!(landed.is_empty(), "nothing submitted apart");
    let room = place_of(&world, pilot).expect("the seller's room");
    crate::driving_rig::teleport(&mut world.rig.tw, buyer, room);

    // 3. The buyer reads the offer, then accepts: one union.
    let (quote, landed) = says(&mut world, buyer, &format!("+core/accept #{}", core.0));
    assert!(landed.is_empty(), "a quote submits nothing");
    assert!(quote.contains(&format!("core #{}", core.0)), "{quote}");
    assert!(quote.contains(head(r::RESALE_HOW)), "{quote}");
    let line = format!("+core/accept #{}=go", core.0);
    let (out, landed) = says(&mut world, buyer, &line);
    assert!(out.contains(head(r::RESALE_ACCEPTING)), "{out}");
    let receipt = landed_one(&line, &landed);
    assert!(receipt.contains("are yours"), "{receipt}");
    let sold = title_of(&world, core);
    assert_eq!(
        sold.title_owner,
        PrincipalId::Object(buyer),
        "the buyer's title"
    );
    assert!(
        sold.encumbrances.is_empty() && sold.scope.is_none(),
        "{sold:?}"
    );
    assert_eq!(sold.revision, title.revision + 1);
    assert_eq!(place_of(&world, core), Some(buyer), "in the buyer's hands");
    assert_eq!(
        accounts(&world),
        [
            before[0] + net,
            before[1] - price,
            before[2],
            before[3] + cut,
            before[4] + tax,
        ],
        "the buyer paid; the seller, the venue and the Registry were paid"
    );
    let receipts: Vec<_> = economy(&world.rig.tw)
        .core_resale_receipts()
        .expect("the audit is complete")
        .cloned()
        .collect();
    assert_eq!(receipts.len(), 1, "{receipts:?}");
    assert_eq!(
        (
            receipts[0].seller,
            receipts[0].buyer,
            receipts[0].core,
            receipts[0].hull,
            receipts[0].price.minor()
        ),
        (pilot, buyer, core, None, price)
    );
    let records = world.rows("core_resales");
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].get("buyer"), Some(&json!(buyer)));
    assert!(world.rows(r::OFFER_FAMILY).is_empty(), "the offer is taken");
    assert!(
        world.rows("core_sales").is_empty(),
        "the buy-back's family is untouched"
    );

    // 4. A restart replays it; the pilot cannot offer the core again, and the buyer can.
    let commit = receipts[0].commit;
    let mut world = world.reboot();
    assert_eq!(title_of(&world, core), sold);
    assert_eq!(place_of(&world, core), Some(buyer));
    assert_eq!(
        economy(&world.rig.tw)
            .core_resale_receipt(commit)
            .expect("the audit is complete")
            .map(|r| (r.core, r.buyer)),
        Some((core, buyer))
    );
    let (out, landed) = says(
        &mut world,
        pilot,
        &format!("+core/offer #{}=#{} price={price}", core.0, third.0),
    );
    assert!(out.contains(r::RESALE_NOT_HELD), "{out}");
    assert!(landed.is_empty());
    let line = format!("+core/offer #{}=#{} price={price}", core.0, third.0);
    let (out, landed) = says(&mut world, buyer, &line);
    assert!(
        landed_one(&line, &landed).contains(head(r::RESALE_OFFERED)),
        "{out}"
    );
    crate::excursions::no_refused_writes(&world.rig.tw);
}

/// **A ship with a governed core fitted sells only with her core, both or neither** (Q-48-52's
/// core leg). After CO.6's route to a cored ship:
///
/// 1. the exchange routes her to the private offer, and the core alone is refused (it is fitted:
///    offer her with it);
/// 2. the captain offers her, with her core, to a buyer;
/// 3. staff lien her: the acceptance is refused whole (the offer's state moved), and neither
///    her title nor her core's moves, nothing paid; released, the captain offers her again;
/// 4. the buyer accepts: in one union her title, her core's title and her hold are the buyer's,
///    the money split as a stored core's; nothing physical moves (the core stays in her); one
///    receipt naming the hull, one record;
/// 5. a restart pairs them and reads the same.
#[test]
fn ex2b_a_cored_ship_sells_with_her_core_both_or_neither() {
    let mut world = crate::co6_cored_ship::world();
    let captain = world.pilot;
    let (core, hull) = crate::co6_cored_ship::core_her(&mut world);
    let buyer = player(&mut world, "Corvina", 9_101);
    fund(&mut world, buyer, 10_000_000);
    let hold = || {
        let w = world.rig.tw.world();
        tacsim::port::ship::hull_view(w, hull)
            .and_then(|view| tacsim::port::ship::hold_vault(w, &view))
            .expect("her hold")
    };
    let hold = hold();
    let vault_owner = |world: &M8World| {
        economy(&world.rig.tw)
            .snapshot()
            .vault(hold)
            .map(|v| v.owner.clone())
    };
    assert_eq!(vault_owner(&world), Some(PrincipalId::Object(captain)));

    // 1. The exchange routes her to the private offer; the core alone is fitted.
    let out = run(
        &mut world.rig.tw,
        captain,
        &format!("+market/post #{} price=5000000", hull.0),
    );
    assert!(
        out.contains(
            &tacsim::port::hull_sale::HULL_CORED
                .replace("{name}", "Salt Moth")
                .replace("{n}", &hull.0.to_string())
        ),
        "{out}"
    );
    let (out, landed) = says(
        &mut world,
        captain,
        &format!("+core/offer #{}=#{} price=5000000", core.0, buyer.0),
    );
    assert!(out.contains(head(r::RESALE_FITTED)), "{out}");
    assert!(landed.is_empty());

    // 2. Her, with her core.
    let price = 6_000_000;
    let offer = format!("+core/offer #{}=#{} price={price}", hull.0, buyer.0);
    let (out, landed) = says(&mut world, captain, &offer);
    assert!(
        landed_one(&offer, &landed).contains("You offer the Salt Moth with her core"),
        "{out}"
    );
    let (her, its) = (title_of(&world, hull), title_of(&world, core));

    // 3. A lien on her: the acceptance is refused whole; nothing moves, nothing is paid.
    god(
        &mut world.rig,
        &format!("@asset/lien #{}=#{} 100 tacsim.credit", hull.0, captain.0),
    );
    let wallets = (credit(&world, captain).1, credit(&world, buyer).1);
    let (out, landed) = says(&mut world, buyer, &format!("+core/accept #{}=go", hull.0));
    assert!(out.contains(r::RESALE_CHANGED), "{out}");
    assert!(landed.is_empty());
    let refused = world.try_perform("tacsim", r::RESALE_ACTION, buyer, json!({"core": core.0}));
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(
        title_of(&world, hull).title_owner,
        PrincipalId::Object(captain)
    );
    assert_eq!(
        title_of(&world, core),
        its,
        "her core's title stays with her"
    );
    assert_eq!(
        (credit(&world, captain).1, credit(&world, buyer).1),
        wallets
    );
    god(&mut world.rig, &format!("@asset/release #{}=1", hull.0));
    let (out, landed) = says(&mut world, captain, &offer);
    let _ = landed_one(&offer, &landed);
    assert!(!out.is_empty());

    // 4. Accepted: both titles and her hold, in one union.
    let venue = world.deployment.venue.recipient.account;
    let levy = world.deployment.registry.account;
    let (cut, tax, net) = split(price);
    let before = (
        credit(&world, captain).1,
        credit(&world, buyer).1,
        balance(&world, venue),
        balance(&world, levy),
    );
    let (quote, landed) = says(&mut world, buyer, &format!("+core/accept #{}", hull.0));
    assert!(landed.is_empty());
    assert!(quote.contains("Salt Moth"), "{quote}");
    let line = format!("+core/accept #{}=go", hull.0);
    let (out, landed) = says(&mut world, buyer, &line);
    let receipt = landed_one(&line, &landed);
    assert!(receipt.contains("both titles"), "{out} / {receipt}");
    for (object, was) in [(hull, &her), (core, &its)] {
        let now = title_of(&world, object);
        assert_eq!(now.title_owner, PrincipalId::Object(buyer), "#{}", object.0);
        assert!(now.encumbrances.is_empty(), "{now:?}");
        assert!(now.revision > was.revision);
    }
    assert_eq!(
        vault_owner(&world),
        Some(PrincipalId::Object(buyer)),
        "her hold"
    );
    assert_eq!(place_of(&world, core), Some(hull), "the core stays in her");
    assert_eq!(
        (
            credit(&world, captain).1,
            credit(&world, buyer).1,
            balance(&world, venue),
            balance(&world, levy),
        ),
        (
            before.0 + net,
            before.1 - price,
            before.2 + cut,
            before.3 + tax
        )
    );
    let receipts: Vec<_> = economy(&world.rig.tw)
        .core_resale_receipts()
        .expect("the audit is complete")
        .cloned()
        .collect();
    assert_eq!(receipts.len(), 1, "{receipts:?}");
    assert_eq!(
        (receipts[0].hull, receipts[0].hold, receipts[0].core),
        (Some(hull), Some(hold), core)
    );
    assert_eq!(world.rows("core_resales").len(), 1);

    // 5. A restart pairs and reads the same.
    let mut world = world.reboot();
    assert_eq!(
        title_of(&world, hull).title_owner,
        PrincipalId::Object(buyer)
    );
    assert_eq!(
        title_of(&world, core).title_owner,
        PrincipalId::Object(buyer)
    );
    assert_eq!(
        economy(&world.rig.tw)
            .snapshot()
            .vault(hold)
            .map(|v| v.owner.clone()),
        Some(PrincipalId::Object(buyer))
    );
    let (out, landed) = says(
        &mut world,
        captain,
        &format!("+core/offer #{}=#{} price={price}", hull.0, buyer.0),
    );
    assert!(out.contains(r::RESALE_NOT_CORE), "{out}");
    assert!(landed.is_empty());
    crate::excursions::no_refused_writes(&world.rig.tw);
}
