//! **Tier 1B security hardening regression tests — port economy, works escrow, holds, and
//! Registry licence disputes** (CRIT-20, HIGH-48, HIGH-50, HIGH-51, HIGH-53).

use mush_core::{GOD, ObjId};
use mush_economy::{ContractState, EconomyStore, VaultId};
use mush_inventory::LotId;
use mush_inventory::record::EndpointRef;
use mush_org::PrincipalId;
use mush_tactics::field_ops::{ComponentCondition, ProvenanceLegalityStatus};
use tacsim::port::works::{WorksContract, WorksState};

use crate::driving_rig::{
    Rig, board, cid, command, drain, fire, hold_pool, hold_vault_of, land, land_all, land_quietly,
    pilot, port_map, prospector, run, seed_hold, seed_lot, seed_part, seed_pool, station_room,
    stocked, store, teleport, typed,
};
use crate::hull_resale_ex2::{mint as mint_minor, owner_of_her};
use crate::in5_production::{ashore, market, sail};
use crate::in10_pools::{days_on, pump, settle};
use crate::pl5_escrow::booked;

const STICKNEY: &str = "stickney_port";
const CERES: &str = "ceres_port";
const METALS: &str = "metals";
const ALLOYS: &str = "structural_alloys";
const FITTINGS: &str = "industrial_parts";
const LICENCE_NAME: &str = "Freeport prospecting license";

fn contract(rig: &Rig, id: u64) -> WorksContract {
    match store(&rig.tw).works_contract(id) {
        Some(WorksState::Present(c)) => (**c).clone(),
        other => panic!("contract {id}: {other:?}"),
    }
}

fn lot_hold_holder(rig: &mut Rig, lot: LotId) -> Option<String> {
    let inventory = rig
        .tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .and_then(|h| h.inventory.clone())
        .expect("inventory");
    typed(&mut rig.tw, &inventory.lot_holds, &lot).map(|h| h.holder().to_owned())
}

fn open_locker_at(rig: &mut Rig, who: ObjId, port: &str, room: &str, venue: &str) -> VaultId {
    let r = station_room(&rig.tw, port, room);
    teleport(&mut rig.tw, who, r);
    let out = run(&mut rig.tw, who, "+locker/open");
    let venue = mush_market::MarketId::new(venue).expect("venue");
    tacsim::port::locker::locker_of(rig.tw.world(), who, &venue)
        .map(|l| VaultId(l.vault))
        .unwrap_or_else(|| panic!("locker at {port}: {out}"))
}

fn crystallize(rig: &mut Rig, who: ObjId, vault: VaultId, good: &str, qty: u64) {
    let out = run(
        &mut rig.tw,
        who,
        &format!("@inv/crystallize #{}/{good}={qty}", vault.0),
    );
    let queued = drain(&mut rig.tw);
    assert!(!queued.is_empty(), "{out}");
    land_all(rig, &queued);
}

/// **CRIT-20**: Works/yard escrow vaults refuse player `@inv/move` (both local and cross-port)
/// and direct market posting/accepting (`MOVE_WORKS_ESCROW`), lot-hold delivered design parts
/// under `tacsim.works/{order}`, and refuse commissioning if any required design part is missing.
#[test]
fn crit20_works_escrow_locks_parts_blocks_player_move_and_market_and_checks_commission() {
    let mut rig = stocked();
    let (captain, row) = owner_of_her(&mut rig, "EscrowTest", 91, "Iron Seal");
    let hold = hold_vault_of(&rig.tw, &row);
    mint_minor(&mut rig, captain, 100_000_000);

    // Open a Ceres locker, then sail captain's Dray to Stickney and open a Stickney locker.
    let ceres_locker = open_locker_at(&mut rig, captain, CERES, "room.c07_exchange", "ceres");
    let ceres_gallery = station_room(&rig.tw, CERES, "room.c01_berth_gallery");
    teleport(&mut rig.tw, captain, ceres_gallery);
    board(&mut rig, captain);
    sail(
        &mut rig,
        captain,
        &row,
        "+ship/undock stickney",
        STICKNEY,
        99_001,
    );
    ashore(&mut rig, captain);
    let stickney_locker =
        open_locker_at(&mut rig, captain, STICKNEY, "room.p08_exchange", "stickney");
    let yard_room = station_room(&rig.tw, STICKNEY, "room.p11_service");
    teleport(&mut rig.tw, captain, yard_room);

    let out = run(&mut rig.tw, captain, "+yard/order tv_escort");
    let order = booked(&out);
    let _ = pump(&mut rig);
    settle(&mut rig, 2);
    let escrow = VaultId(order);

    // Seed a clean design part in the Dray's hold and deliver it via `+yard/deliver`.
    let sensor_lot = LotId(99_101);
    let sensor = "tacsim.part.sensor.visual_mk1";
    seed_part(
        &mut rig,
        hold,
        sensor,
        sensor_lot,
        ComponentCondition::Intact,
        ProvenanceLegalityStatus::CleanTitle,
    );
    settle(&mut rig, 1);
    let out = run(
        &mut rig.tw,
        captain,
        &format!("+yard/deliver {order}={sensor} 1"),
    );
    assert!(out.contains("moves into order"), "{out}");
    settle(&mut rig, 2);

    // 1. Delivered design part is lot-held under `tacsim.works/{order}`!
    assert_eq!(
        lot_hold_holder(&mut rig, sensor_lot).as_deref(),
        Some(format!("tacsim.works/{order}").as_str()),
        "delivered yard part must be lot-held by the works contract"
    );

    // Seed bulk alloys into escrow and locker to test `@inv/move` in both directions.
    seed_pool(&mut rig, escrow, ALLOYS, 3, LotId(99_102));
    seed_pool(&mut rig, stickney_locker, ALLOYS, 2, LotId(99_103));

    // 2. Player `@inv/move` OUT of works escrow (same port) is refused with MOVE_WORKS_ESCROW.
    let exchange_room = station_room(&rig.tw, STICKNEY, "room.p08_exchange");
    teleport(&mut rig.tw, captain, exchange_room);
    let (out, queued) = command(
        &mut rig,
        captain,
        &format!("@inv/move {}/{ALLOYS}:1={}", escrow.0, stickney_locker.0),
    );
    assert!(queued.is_empty(), "must refuse before queueing: {out}");
    assert!(
        out.contains(tacsim::port::market::MOVE_WORKS_ESCROW),
        "expected MOVE_WORKS_ESCROW, got: {out}"
    );
    assert_eq!(hold_pool(&mut rig.tw, escrow, ALLOYS), 3);

    // 3. Player `@inv/move` INTO works escrow is refused with MOVE_WORKS_ESCROW.
    let (out, queued) = command(
        &mut rig,
        captain,
        &format!("@inv/move {}/{ALLOYS}:1={}", stickney_locker.0, escrow.0),
    );
    assert!(queued.is_empty(), "must refuse before queueing: {out}");
    assert!(
        out.contains(tacsim::port::market::MOVE_WORKS_ESCROW),
        "expected MOVE_WORKS_ESCROW, got: {out}"
    );

    // 4. Cross-port teleport via `@inv/move` out of works escrow into Ceres locker is refused.
    let (out, queued) = command(
        &mut rig,
        captain,
        &format!("@inv/move {}/{ALLOYS}:1={}", escrow.0, ceres_locker.0),
    );
    assert!(queued.is_empty(), "must refuse cross-port move: {out}");
    assert!(
        out.contains(tacsim::port::market::MOVE_WORKS_ESCROW),
        "{out}"
    );

    // 5. Direct market posting of a lot sitting in a works escrow is refused by PortVenueGate.
    let escrow_alloy_lot = LotId(99_104);
    seed_lot(&mut rig, escrow, ALLOYS, 1, escrow_alloy_lot);
    let post = run(
        &mut rig.tw,
        captain,
        &format!("+market/post {escrow_alloy_lot} price=1000"),
    );
    assert!(
        post.contains(tacsim::port::market::MOVE_WORKS_ESCROW),
        "direct market post from escrow must be refused: {post}"
    );

    // 6. Direct `+market/accept` targeting a works escrow vault is refused by PortVenueGate.
    let kasei_ask = {
        let kasei = port_map(&rig.tw, STICKNEY).vendors[&cid("kasei")].vault;
        market(&rig)
            .lot_listing_of(EndpointRef(VaultId(kasei)), &cid(ALLOYS))
            .expect("Kasei asks alloys")
    };
    let quoted = run(
        &mut rig.tw,
        captain,
        &format!("+market/quote {kasei_ask}=1"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let qid = market(&rig)
        .quotes_of_buyer(&PrincipalId::Object(captain))
        .map(|q| q.id)
        .max()
        .expect("quote");
    let accepted = run(
        &mut rig.tw,
        captain,
        &format!("+market/accept {qid}=#{}", escrow.0),
    );
    assert!(
        accepted.contains(tacsim::port::market::MOVE_WORKS_ESCROW),
        "direct market accept into escrow must be refused: {accepted}"
    );

    // 7. Missing/unclean required design parts refuse `+yard/collect` commissioning even once the
    // hull kit completes.
    teleport(&mut rig.tw, captain, yard_room);
    let order2 = booked(&run(&mut rig.tw, captain, "+yard/order tv_escort"));
    let _ = pump(&mut rig);
    settle(&mut rig, 2);
    let escrow2 = VaultId(order2);
    seed_pool(&mut rig, escrow2, ALLOYS, 3, LotId(99_200));
    seed_pool(&mut rig, escrow2, FITTINGS, 1, LotId(99_201));
    for (i, part) in [
        "tacsim.part.weapon.cannon_105",
        "tacsim.part.weapon.mg_coax",
        "tacsim.part.plant.vehicle_mk1",
        "tacsim.part.sensor.visual_mk1",
        "tacsim.part.core.line",
    ]
    .iter()
    .enumerate()
    {
        seed_lot(&mut rig, escrow2, part, 1, LotId(99_210 + i as u64));
    }
    settle(&mut rig, 2);
    days_on(&mut rig, 3);
    // Verify the yard design part was lot-held during the build; attempt GOD move and if held,
    // mark one part Damaged in provenance to verify `commission()` refuses incomplete clean parts.
    let inventory = rig
        .tw
        .world()
        .module_state::<tacsim::port::bay_work::act::BayActHandles>()
        .and_then(|h| h.inventory.clone())
        .expect("inventory");
    let (_out, queued) = command(
        &mut rig,
        GOD,
        &format!("@inv/move {}/lot:99210={}", escrow2.0, hold.0),
    );
    if !queued.is_empty() {
        let _ = land(&mut rig, &queued[0]);
    }
    let still_in_escrow = typed(&mut rig.tw, &inventory.lots, &LotId(99_210))
        .is_some_and(|l| l.endpoint() == EndpointRef(escrow2));
    if still_in_escrow {
        assert_eq!(
            lot_hold_holder(&mut rig, LotId(99_210)).as_deref(),
            Some(format!("tacsim.works/{order2}").as_str()),
            "yard design part lot was locked during build"
        );
        let parts_family = rig
            .tw
            .world()
            .module_state::<tacsim::port::bay_work::act::BayActHandles>()
            .map(|h| h.parts.clone())
            .expect("parts");
        let definition = store(&rig.tw)
            .definitions()
            .parts
            .values()
            .find(|r| r.commodity.as_str() == "tacsim.part.weapon.cannon_105")
            .map(|r| r.row.clone())
            .expect("cannon def");
        crate::driving_rig::reseed_typed(
            &mut rig,
            &parts_family,
            &LotId(99_210),
            tacsim::port::bay_work::act::PartProvenance {
                lot: LotId(99_210),
                definition,
                condition: ComponentCondition::Damaged,
                legality: ProvenanceLegalityStatus::CleanTitle,
                fitted_into: None,
                origin: tacsim::port::bay_work::act::PartOrigin::Seeded {
                    receipt: "test:crit20".to_owned(),
                },
            },
        );
    }
    let collect_out = run(&mut rig.tw, captain, &format!("+yard/collect {order2}"));
    assert!(
        collect_out.contains("does not carry all required clean parts"),
        "expected missing/unclean parts refusal, got: {collect_out}"
    );
    assert!(
        tacsim::port::commission::commission(rig.tw.world(), order2).is_none(),
        "no YardCommission created when required clean parts are missing"
    );
}

/// **HIGH-48**: Crystallized divisible lots in a chartered hold are counted by `read_held`, so
/// both `+coop/release` (`RECALL_CARGO`) and `+coop/refit` (`REFIT_HOLD`) refuse while a
/// crystallized divisible lot sits in the hold (even though the raw pool is 0).
#[test]
fn high48_crystallized_divisible_lot_blocks_coop_release_and_refit() {
    let mut rig = stocked();
    let (pilot_actor, row) = prospector(&mut rig, "CrystalPilot", 92, "Salt Tern");
    let hold = hold_vault_of(&rig.tw, &row);
    seed_hold(&mut rig, &row, METALS, 5, 99_300);
    crystallize(&mut rig, pilot_actor, hold, METALS, 5);
    assert_eq!(
        hold_pool(&mut rig.tw, hold, METALS),
        0,
        "pool reads 0 after crystallize"
    );

    // 1. `+coop/release` refuses with `RECALL_CARGO`.
    let release = run(&mut rig.tw, pilot_actor, "+coop/release");
    assert!(
        release.contains(&tacsim::port::recall::RECALL_CARGO.replace("{name}", "Salt Tern")),
        "expected RECALL_CARGO with crystallized lot in hold, got: {release}"
    );
    assert!(
        tacsim::port::live_charter(rig.tw.world(), pilot_actor).is_some(),
        "charter remains active"
    );

    // 2. `+coop/refit standard` refuses with `REFIT_HOLD`.
    let refit = run(&mut rig.tw, pilot_actor, "+coop/refit standard");
    assert!(
        refit.contains(&tacsim::port::refit::REFIT_HOLD.replace("{name}", "Salt Tern")),
        "expected REFIT_HOLD with crystallized lot in hold, got: {refit}"
    );
}

/// **HIGH-50**: `RegistryFeeDisputeGuard` blocks non-admin players from disputing a prospecting
/// licence fee contract created by `+license/buy go`.
#[test]
fn high50_licence_fee_dispute_is_refused_by_guard() {
    let mut rig = stocked();
    let (pilot_actor, _row) = prospector(&mut rig, "LicPilot", 93, "Grey Gull");
    mint_minor(&mut rig, pilot_actor, 10_000_000);
    let out = run(&mut rig.tw, pilot_actor, "+license/buy go");
    assert!(!out.contains("cannot"), "{out}");

    let fee_id = {
        let econ = rig
            .tw
            .world()
            .module_state::<EconomyStore>()
            .expect("economy");
        econ.snapshot()
            .contracts()
            .find(|c| {
                c.issuer == PrincipalId::Object(pilot_actor) && c.name.as_str() == LICENCE_NAME
            })
            .map(|c| c.id)
            .expect("licence fee contract")
    };
    let state_before = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(fee_id).map(|c| c.state));
    assert!(
        matches!(
            state_before,
            Some(ContractState::Active | ContractState::ReadyToSettle)
        ),
        "licence fee contract is live: {state_before:?}"
    );

    let refused = run(
        &mut rig.tw,
        pilot_actor,
        &format!("@contract/dispute #{}", fee_id.0),
    );
    assert!(
        refused.contains(tacsim::port::claims::REGISTRY_FEE_DISPUTE_REFUSED),
        "expected RegistryFeeDisputeGuard refusal on licence fee, got: {refused}"
    );
    let state_after = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(fee_id).map(|c| c.state));
    assert_ne!(state_after, Some(ContractState::Disputed));
}

/// **HIGH-51**: `works::book` pre-checks affordability and the per-actor open contract cap
/// (`MAX_OPEN_CONTRACTS_PER_ACTOR = 16`) BEFORE minting any `Thing` or `Vault`.
#[test]
fn high51_works_book_prechecks_prevent_object_vault_leaks_and_cap_open_orders() {
    let mut rig = stocked();
    settle(&mut rig, 40);
    let works_room = station_room(&rig.tw, STICKNEY, "room.p08_exchange");

    // 1. Unfunded player: 0 credits, refused BEFORE creating any Thing or Vault!
    let broke = pilot(&mut rig, "Broke", 94);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", broke.0),
    );
    teleport(&mut rig.tw, broke, works_room);
    let next_obj_before = rig.tw.world().next_id();
    let vaults_before = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .expect("econ")
        .snapshot()
        .vaults()
        .count();
    let refused = run(&mut rig.tw, broke, "+works/order fittings 1");
    assert!(
        refused.contains("cannot cover the booking fee"),
        "expected affordability refusal: {refused}"
    );
    assert_eq!(
        rig.tw.world().next_id(),
        next_obj_before,
        "no Thing allocated on unfunded booking"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<EconomyStore>()
            .expect("econ")
            .snapshot()
            .vaults()
            .count(),
        vaults_before,
        "no Vault allocated on unfunded booking"
    );

    // 2. Funded player caps at `MAX_OPEN_CONTRACTS_PER_ACTOR` (16); the 17th refuses cleanly
    // without leaking a Thing or Vault.
    let rich = pilot(&mut rig, "Rich", 95);
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!("@account/open #{}=tacsim.credit", rich.0),
    );
    mint_minor(&mut rig, rich, 100_000_000);
    teleport(&mut rig.tw, rich, works_room);
    for i in 0..tacsim::port::works::MAX_OPEN_CONTRACTS_PER_ACTOR {
        let out = run(&mut rig.tw, rich, "+works/order fittings 1");
        assert!(out.contains("Order "), "booking #{i} failed: {out}");
        let _ = pump(&mut rig);
        settle(&mut rig, 1);
    }
    let next_obj_at_cap = rig.tw.world().next_id();
    let vaults_at_cap = rig
        .tw
        .world()
        .module_state::<EconomyStore>()
        .expect("econ")
        .snapshot()
        .vaults()
        .count();
    let capped = run(&mut rig.tw, rich, "+works/order fittings 1");
    assert!(
        capped.contains("cap is"),
        "expected open contract cap refusal, got: {capped}"
    );
    assert_eq!(
        rig.tw.world().next_id(),
        next_obj_at_cap,
        "no Thing allocated at cap"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<EconomyStore>()
            .expect("econ")
            .snapshot()
            .vaults()
            .count(),
        vaults_at_cap,
        "no Vault allocated at cap"
    );
}

/// **HIGH-53**: `WorksProvider::room_for` accumulates a multi-good batch overlay so collecting
/// two commodities that each fit alone (3 t + 3 t vs 5 t free) is refused without overflowing
/// the docked hold.
#[test]
fn high53_works_collect_rejects_multi_good_hold_capacity_overflow() {
    let mut rig = stocked();
    settle(&mut rig, 40);
    let (captain, row) = owner_of_her(&mut rig, "BatchCap", 96, "Heavy Ox");
    let hold = hold_vault_of(&rig.tw, &row);
    mint_minor(&mut rig, captain, 50_000_000);

    // Sail captain's Dray (100 t hold) to Stickney and fill it to 95 t (5 t free).
    board(&mut rig, captain);
    sail(
        &mut rig,
        captain,
        &row,
        "+ship/undock stickney",
        STICKNEY,
        99_002,
    );
    ashore(&mut rig, captain);
    // Clear any leftover voyage propellant/provisions mass before seeding exact 95 t.
    let used_mass_t = |rig: &mut Rig| {
        hold_pool(&mut rig.tw, hold, "propellant") + hold_pool(&mut rig.tw, hold, "provisions")
    };
    let fill_metals = 95_u64.saturating_sub(used_mass_t(&mut rig));
    if fill_metals > 0 {
        seed_hold(&mut rig, &row, METALS, fill_metals, 99_400);
    }

    let works_room = station_room(&rig.tw, STICKNEY, "room.p08_exchange");
    teleport(&mut rig.tw, captain, works_room);
    let out = run(&mut rig.tw, captain, "+works/order fittings 1");
    let order = booked(&out);
    let _ = pump(&mut rig);
    settle(&mut rig, 2);

    // Seed TWO commodities into the escrow: 3 t alloys + 3 t fittings = 6 t (> 5 t free, though
    // each 3 t fits individually if the batch overlay were missing!).
    let escrow = VaultId(order);
    seed_pool(&mut rig, escrow, ALLOYS, 3, LotId(99_401));
    seed_pool(&mut rig, escrow, FITTINGS, 3, LotId(99_402));

    // Cancel the order so its escrow tries to return into the 95 t-full hold.
    let _ = run(&mut rig.tw, captain, &format!("+works/cancel {order}"));
    let _ = pump(&mut rig);
    for _ in 0..8 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        let _ = pump(&mut rig);
    }
    let _ = run(&mut rig.tw, captain, &format!("+works/collect {order}"));
    for _ in 0..8 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
        let _ = pump(&mut rig);
    }
    assert_eq!(hold_pool(&mut rig.tw, hold, ALLOYS), 0);
    assert_eq!(hold_pool(&mut rig.tw, hold, FITTINGS), 0);
    assert_eq!(hold_pool(&mut rig.tw, escrow, ALLOYS), 3);
    assert_eq!(hold_pool(&mut rig.tw, escrow, FITTINGS), 3);
    assert!(
        contract(&rig, order).closed.is_none(),
        "order remains open with goods still in escrow"
    );
}

/// **EXT-P2-10**: `PortVenueGate::receives_at` refuses delivery into a governed hull hold, a
/// works escrow vault, or a foreign port locker when the target venue is a non-port venue.
#[test]
fn ext_p2_10_non_port_venue_receives_at_rejects_hold_escrow_and_foreign_locker() {
    use mush_market::{MarketId, VenueGate};

    let mut rig = stocked();
    let (captain, row) = owner_of_her(&mut rig, "PlacedGate", 97, "Dark Gull");
    let hold = hold_vault_of(&rig.tw, &row);
    mint_minor(&mut rig, captain, 100_000_000);

    let ceres_locker = open_locker_at(&mut rig, captain, CERES, "room.c07_exchange", "ceres");
    let ceres_gallery = station_room(&rig.tw, CERES, "room.c01_berth_gallery");
    teleport(&mut rig.tw, captain, ceres_gallery);
    board(&mut rig, captain);
    sail(
        &mut rig,
        captain,
        &row,
        "+ship/undock stickney",
        STICKNEY,
        99_003,
    );
    ashore(&mut rig, captain);
    let yard_room = station_room(&rig.tw, STICKNEY, "room.p11_service");
    teleport(&mut rig.tw, captain, yard_room);
    let order = booked(&run(&mut rig.tw, captain, "+yard/order tv_escort"));
    let _ = pump(&mut rig);
    settle(&mut rig, 2);
    let escrow = VaultId(order);

    let non_port = MarketId::new("launch.counter").expect("non-port venue");
    let gate = tacsim::port::market::PortVenueGate;
    let w = rig.tw.world();

    // 1. Governed hold docked at Stickney cannot receive delivery at a non-port venue.
    let hold_err = gate
        .receives_at(w, &non_port, EndpointRef(hold))
        .expect_err("governed hold must be refused on non-port venue");
    assert!(
        hold_err.line.contains("Dark Gull") && hold_err.line.contains("launch.counter"),
        "expected HOLD_ELSEWHERE naming hull and venue, got: {}",
        hold_err.line
    );

    // 2. Works escrow cannot receive delivery at a non-port venue.
    let escrow_err = gate
        .receives_at(w, &non_port, EndpointRef(escrow))
        .expect_err("works escrow must be refused on non-port venue");
    assert_eq!(escrow_err.line, tacsim::port::market::MOVE_WORKS_ESCROW);

    // 3. Foreign port locker cannot receive delivery at a non-port venue.
    let locker_err = gate
        .receives_at(w, &non_port, EndpointRef(ceres_locker))
        .expect_err("foreign locker must be refused on non-port venue");
    assert_eq!(
        locker_err.line,
        tacsim::port::market::LOCKER_ELSEWHERE.replace("{port}", "launch.counter")
    );

    // 4. Same governed hold and Ceres locker still behave properly at their own ports.
    let stickney = MarketId::new("stickney").expect("stickney");
    let ceres = MarketId::new("ceres").expect("ceres");
    assert!(gate.receives_at(w, &stickney, EndpointRef(hold)).is_ok());
    assert!(
        gate.receives_at(w, &ceres, EndpointRef(ceres_locker))
            .is_ok()
    );
}

/// **P2-TSW-2**: Scrap vaults and facility stores vaults enforce fail-closed venue locality in
/// `PortVenueGate::admit`, `PortVenueGate::receives_at`, and `PortMovePolicy::admit` (while
/// preserving local scrap withdrawal and local idle facility stores use, and blocking moves or
/// deliveries into facility stores while `asset_on_sale`).
#[test]
#[allow(clippy::too_many_lines)]
fn p2_tsw_2_container_vaults_enforce_cross_port_and_on_sale_locality() {
    use mush_market::{MarketId, Presence, VenueGate};

    let mut rig = stocked();
    let (captain, _row) = owner_of_her(&mut rig, "VaultLocal", 101, "Copper Fin");
    mint_minor(&mut rig, captain, 100_000_000);

    let ceres_locker = open_locker_at(&mut rig, captain, CERES, "room.c07_exchange", "ceres");
    let stickney_locker =
        open_locker_at(&mut rig, captain, STICKNEY, "room.p08_exchange", "stickney");

    // Create a Ceres scrap vault for `captain` with 4 t alloys + 1 alloys lot.
    let scrap_crate = rig
        .tw
        .world_mut()
        .create(
            mush_core::ObjKind::Thing,
            "scrap at ceres",
            captain,
            Some(mush_core::LIMBO),
        )
        .expect("scrap crate");
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@vault/create scrap at ceres=container:#{} owner=#{}",
            scrap_crate.0, captain.0
        ),
    );
    let ceres_scrap = tacsim::port::bay_work::scrap::scrap_vault(rig.tw.world(), captain, "ceres")
        .expect("ceres scrap vault");
    seed_pool(&mut rig, ceres_scrap, ALLOYS, 4, LotId(99_501));
    let scrap_lot = LotId(99_502);
    seed_lot(&mut rig, ceres_scrap, ALLOYS, 1, scrap_lot);

    // 1. Local scrap withdrawal at Ceres succeeds; moving INTO scrap at Ceres fails.
    let ceres_exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, captain, ceres_exchange);
    let (out, queued) = command(
        &mut rig,
        captain,
        &format!("@inv/move {}/{ALLOYS}:1={}", ceres_scrap.0, ceres_locker.0),
    );
    assert!(
        !queued.is_empty(),
        "local scrap withdrawal must work: {out}"
    );
    land_all(&mut rig, &queued);
    assert_eq!(hold_pool(&mut rig.tw, ceres_locker, ALLOYS), 1);

    let (out, queued) = command(
        &mut rig,
        captain,
        &format!("@inv/move {}/{ALLOYS}:1={}", ceres_locker.0, ceres_scrap.0),
    );
    assert!(queued.is_empty(), "move into scrap must fail: {out}");
    assert!(
        out.contains(tacsim::port::market::MOVE_INTO_SCRAP),
        "expected MOVE_INTO_SCRAP, got: {out}"
    );

    // 2. Cross-port access to Ceres scrap vault from Stickney fails closed in `@inv/move` and
    // `+market/post`.
    let stickney_exchange = station_room(&rig.tw, STICKNEY, "room.p08_exchange");
    teleport(&mut rig.tw, captain, stickney_exchange);
    let (out, queued) = command(
        &mut rig,
        captain,
        &format!(
            "@inv/move {}/{ALLOYS}:1={}",
            ceres_scrap.0, stickney_locker.0
        ),
    );
    assert!(queued.is_empty(), "cross-port scrap move must fail: {out}");
    assert!(
        out.contains(&tacsim::port::market::SCRAP_ELSEWHERE.replace("{port}", "Stickney Yards")),
        "expected SCRAP_ELSEWHERE, got: {out}"
    );
    assert_eq!(hold_pool(&mut rig.tw, stickney_locker, ALLOYS), 0);

    let post_out = run(
        &mut rig.tw,
        captain,
        &format!("+market/post {scrap_lot} price=50000"),
    );
    assert!(
        post_out
            .contains(&tacsim::port::market::SCRAP_ELSEWHERE.replace("{port}", "Stickney Yards")),
        "cross-port post from scrap vault must fail: {post_out}"
    );

    // 3. Facility stores vault: give `captain` title and stores of Stickney's yard (`p12_observation`).
    let yard = station_room(&rig.tw, STICKNEY, "room.p12_observation");
    let yards_org = mush_org::OrgId(port_map(&rig.tw, STICKNEY).orgs[&cid("stickney_yards")].org);
    let root = port_map(&rig.tw, STICKNEY).orgs[&cid("stickney_yards")].root_scope;
    let _ = run(
        &mut rig.tw,
        GOD,
        &format!(
            "@vault/create YardStores=container:#{} owner=org:{} scope={root}",
            yard.0, yards_org.0
        ),
    );
    let yard_stores =
        tacsim::port::facility_sale::stores_of(rig.tw.world(), yard).expect("stickney yard stores");
    let transferred = run(
        &mut rig.tw,
        GOD,
        &format!("@asset/transfer #{}=#{}", yard.0, captain.0),
    );
    assert!(!transferred.contains("Usage"), "{transferred}");
    {
        let engine = rig.tw.engine_mut();
        let mut out = mush_core::Outbox::new();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(tacsim::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        tacsim::port::facility_sale::on_sold(
            &mut ctx,
            yard,
            &PrincipalId::Organization(yards_org),
            &PrincipalId::Object(captain),
        );
    }
    seed_pool(&mut rig, stickney_locker, METALS, 2, LotId(99_503));

    // Local move into and out of Stickney yard stores at Stickney succeeds when not on sale.
    let (out, queued) = command(
        &mut rig,
        captain,
        &format!(
            "@inv/move {}/{METALS}:2={}",
            stickney_locker.0, yard_stores.0
        ),
    );
    assert!(!queued.is_empty(), "local move into yard stores: {out}");
    land_all(&mut rig, &queued);
    assert_eq!(hold_pool(&mut rig.tw, yard_stores, METALS), 2);

    // Cross-port move & market access to Stickney yard stores from Ceres MUST fail with STORES_ELSEWHERE!
    teleport(&mut rig.tw, captain, ceres_exchange);
    let (out, queued) = command(
        &mut rig,
        captain,
        &format!("@inv/move {}/{METALS}:1={}", yard_stores.0, ceres_locker.0),
    );
    assert!(
        queued.is_empty(),
        "cross-port move out of yard stores: {out}"
    );
    assert!(
        out.contains(&tacsim::port::market::STORES_ELSEWHERE.replace("{port}", "Ceres Freeport")),
        "expected STORES_ELSEWHERE, got: {out}"
    );
    assert_eq!(hold_pool(&mut rig.tw, ceres_locker, METALS), 0);

    let ceres_ask = {
        let exch = port_map(&rig.tw, CERES).vendors[&cid("freeport_exchange")].vault;
        market(&rig)
            .lot_listing_of(EndpointRef(VaultId(exch)), &cid(METALS))
            .expect("Ceres metals ask")
    };
    let quoted = run(
        &mut rig.tw,
        captain,
        &format!("+market/quote {ceres_ask}=1"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let qid = market(&rig)
        .quotes_of_buyer(&PrincipalId::Object(captain))
        .map(|q| q.id)
        .max()
        .expect("quote");
    let accepted = run(
        &mut rig.tw,
        captain,
        &format!("+market/accept {qid}={}", yard_stores.0),
    );
    assert!(
        accepted
            .contains(&tacsim::port::market::STORES_ELSEWHERE.replace("{port}", "Ceres Freeport")),
        "cross-port accept into foreign facility stores must fail: {accepted}"
    );

    let bid_out = run(
        &mut rig.tw,
        captain,
        &format!(
            "+market/bid tacsim:{METALS} qty=1 price=1000 into={}",
            yard_stores.0
        ),
    );
    assert!(
        bid_out
            .contains(&tacsim::port::market::STORES_ELSEWHERE.replace("{port}", "Ceres Freeport")),
        "cross-port bid into foreign facility stores must fail: {bid_out}"
    );

    // Also verify `PortVenueGate::receives_at` rejects both scrap and foreign facility stores
    // at Ceres and at non-port `launch.counter`.
    let gate = tacsim::port::market::PortVenueGate;
    let ceres_id = MarketId::new("ceres").expect("ceres");
    let stickney_id = MarketId::new("stickney").expect("stickney");
    let launch_id = MarketId::new("launch.counter").expect("launch");
    assert!(
        gate.receives_at(rig.tw.world(), &ceres_id, EndpointRef(yard_stores))
            .is_err()
    );
    assert!(
        gate.receives_at(rig.tw.world(), &launch_id, EndpointRef(yard_stores))
            .is_err()
    );
    assert!(
        gate.receives_at(rig.tw.world(), &launch_id, EndpointRef(ceres_scrap))
            .is_err()
    );
    assert!(
        gate.admit(
            rig.tw.world(),
            captain,
            &ceres_id,
            Some(EndpointRef(yard_stores)),
            Presence::Here
        )
        .is_err()
    );

    // 4. Empty the Stickney yard stores at Stickney, list the yard for sale, and verify
    // `@inv/move` and `receives_at` refuse moving/delivering goods into its stores while on sale!
    teleport(&mut rig.tw, captain, stickney_exchange);
    let (out, queued) = command(
        &mut rig,
        captain,
        &format!(
            "@inv/move {}/{METALS}:2={}",
            yard_stores.0, stickney_locker.0
        ),
    );
    assert!(!queued.is_empty(), "empty yard stores at Stickney: {out}");
    land_all(&mut rig, &queued);
    assert_eq!(hold_pool(&mut rig.tw, yard_stores, METALS), 0);

    let listed = run(
        &mut rig.tw,
        captain,
        &format!("+market/post #{} price=1000000", yard.0),
    );
    assert!(listed.contains("up for sale as #"), "{listed}");
    assert!(mush_market::asset_on_sale(rig.tw.world(), yard));

    let yard_name = rig
        .tw
        .world()
        .get(yard)
        .map(|o| o.name.to_string())
        .expect("yard name");
    let expected_on_sale =
        tacsim::port::market::MOVE_FACILITY_ON_SALE.replace("{name}", &yard_name);

    let (out, queued) = command(
        &mut rig,
        captain,
        &format!(
            "@inv/move {}/{METALS}:1={}",
            stickney_locker.0, yard_stores.0
        ),
    );
    assert!(
        queued.is_empty(),
        "move into on-sale facility stores: {out}"
    );
    assert!(
        out.contains(&expected_on_sale),
        "expected MOVE_FACILITY_ON_SALE, got: {out}"
    );

    let err = gate
        .receives_at(rig.tw.world(), &stickney_id, EndpointRef(yard_stores))
        .expect_err("receives_at into on-sale facility stores must fail");
    assert_eq!(err.line, expected_on_sale);
}

/// **P2-TSW-3**: `PortVenueGate::receives_at` and `hull_admits` reject governed holds that are
/// cleared-to-sail (`pending_of`) or listed-for-sale (`asset_on_sale`), `in_flight_for` blocks
/// listing a hull while an open `Supply::Wanted` bid targets its hold, and `sell_into_bid`
/// re-verifies live bidder vault control so post-transfer cargo injection is blocked.
#[test]
#[allow(clippy::too_many_lines)]
fn p2_tsw_3_governed_hold_blocks_cleared_on_sale_and_post_transfer_bid_injection() {
    use mush_market::{MarketId, Presence, VenueGate};

    let mut rig = stocked();
    let (captain, row) = owner_of_her(&mut rig, "BidGuard", 102, "Swift Tern");
    let hull = ObjId(row.hull);
    let hold = hold_vault_of(&rig.tw, &row);
    mint_minor(&mut rig, captain, 100_000_000);

    let ceres_id = MarketId::new("ceres").expect("ceres");
    let gate = tacsim::port::market::PortVenueGate;
    let ceres_exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, captain, ceres_exchange);

    // 1. Listed-for-sale (`asset_on_sale`) blocks both `hull_admits` (`+market/accept` into hold)
    // and `PortVenueGate::receives_at` with `MOVE_HULL_ON_SALE`.
    let listing_id = crate::hull_resale_ex2::list(&mut rig, captain, hull);
    assert!(mush_market::asset_on_sale(rig.tw.world(), hull));
    let expected_on_sale = tacsim::port::market::MOVE_HULL_ON_SALE.replace("{name}", "Swift Tern");

    let recv_err = gate
        .receives_at(rig.tw.world(), &ceres_id, EndpointRef(hold))
        .expect_err("receives_at must reject on-sale hull hold");
    assert_eq!(recv_err.line, expected_on_sale);

    let admit_err = gate
        .admit(
            rig.tw.world(),
            captain,
            &ceres_id,
            Some(EndpointRef(hold)),
            Presence::Here,
        )
        .expect_err("admit must reject on-sale hull hold");
    assert_eq!(admit_err.line, expected_on_sale);

    // `+market/accept` targeting the listed hull's hold fails before any contract is created.
    let ceres_metals = {
        let exch = port_map(&rig.tw, CERES).vendors[&cid("freeport_exchange")].vault;
        market(&rig)
            .lot_listing_of(EndpointRef(VaultId(exch)), &cid(METALS))
            .expect("Ceres metals ask")
    };
    let quoted = run(
        &mut rig.tw,
        captain,
        &format!("+market/quote {ceres_metals}=1"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let qid = market(&rig)
        .quotes_of_buyer(&PrincipalId::Object(captain))
        .map(|q| q.id)
        .max()
        .expect("quote");
    let accepted = run(
        &mut rig.tw,
        captain,
        &format!("+market/accept {qid}={}", hold.0),
    );
    assert!(
        accepted.contains(&expected_on_sale),
        "accept into listed hull hold must fail: {accepted}"
    );

    // Withdraw the hull listing so we can test open-bid and cleared-to-sail checks.
    let withdrawn = run(
        &mut rig.tw,
        captain,
        &format!("+market/withdraw {}", listing_id.0),
    );
    assert!(
        !mush_market::asset_on_sale(rig.tw.world(), hull),
        "hull listing withdrawn: {withdrawn}"
    );

    // 2. Open indivisible-part `Supply::Wanted` bid into `hold` blocks `+market/post #<hull>` via
    // `in_flight_for`, and if `hold` is later re-homed to Buyer B, `sell_into_bid` refuses!
    let sensor = "tacsim.part.sensor.visual_mk1";
    let bid_out = run(
        &mut rig.tw,
        captain,
        &format!(
            "+market/bid tacsim:{sensor} qty=1 price=50000 into={}",
            hold.0
        ),
    );
    assert!(bid_out.contains("Bid #"), "{bid_out}");
    let bid_id = bid_out
        .split("Bid #")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse::<u64>().ok())
        .expect("bid id");

    let list_blocked = run(
        &mut rig.tw,
        captain,
        &format!("+market/post #{} price=6000000", hull.0),
    );
    assert!(
        !list_blocked.contains("up for sale as #"),
        "open bid targeting hull hold must block listing hull: {list_blocked}"
    );

    // Prepare a seller with a clean sensor part ready to sell into `bid_id`.
    let (seller, seller_row) = owner_of_her(&mut rig, "PartSeller", 103, "Iron Wren");
    let seller_hold = hold_vault_of(&rig.tw, &seller_row);
    let sensor_lot = LotId(99_601);
    seed_part(
        &mut rig,
        seller_hold,
        sensor,
        sensor_lot,
        ComponentCondition::Intact,
        ProvenanceLegalityStatus::CleanTitle,
    );
    teleport(&mut rig.tw, seller, ceres_exchange);

    // 3. File `+ship/undock stickney` (cleared to sail) on a second hull and verify `receives_at`
    // refuses with `HOLD_CLEARED`.
    let (sailor, sailor_row) = owner_of_her(&mut rig, "ClearedShip", 104, "Wind Dart");
    let sailor_hull = ObjId(sailor_row.hull);
    let sailor_hold = hold_vault_of(&rig.tw, &sailor_row);
    seed_hold(&mut rig, &sailor_row, "propellant", 80, 99_602);
    seed_hold(&mut rig, &sailor_row, "provisions", 100, 99_603);
    let ceres_gallery = station_room(&rig.tw, CERES, "room.c01_berth_gallery");
    teleport(&mut rig.tw, sailor, ceres_gallery);
    board(&mut rig, sailor);
    let undock = run(&mut rig.tw, sailor, "+ship/undock stickney");
    let _ = drain(&mut rig.tw);
    assert!(
        tacsim::port::voyage::pending_of(rig.tw.world(), sailor_hull).is_some(),
        "hull must have a pending clearance: {undock}"
    );
    let cleared_err = gate
        .receives_at(rig.tw.world(), &ceres_id, EndpointRef(sailor_hold))
        .expect_err("receives_at must reject cleared-to-sail hull hold");
    assert_eq!(
        cleared_err.line,
        tacsim::port::market::HOLD_CLEARED
            .replace("{name}", "Wind Dart")
            .replace("{port}", "Ceres Freeport")
    );

    // 4. Re-home `captain`'s `hold` to a new owner `buyer_b` (simulating `+hull/sell` / `+coop/release`):
    // `PartSeller` trying to fill `bid_id` MUST be refused with `BIDDER_NOT_BUYING`!
    let buyer_b = pilot(&mut rig, "NewHullOwner", 105);
    {
        let engine = rig.tw.engine_mut();
        let mut out = mush_core::Outbox::new();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(tacsim::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        assert!(mush_economy::rehome_vault(
            &mut ctx,
            hold,
            PrincipalId::Object(buyer_b),
            None,
            smol_str::SmolStr::new("Swift Tern hold"),
        ));
    }
    let (sold, queued) = command(
        &mut rig,
        seller,
        &format!("+market/sell {bid_id}=1 from={}", seller_hold.0),
    );
    assert!(
        queued.is_empty(),
        "must not deliver into re-homed hold: {sold}"
    );
    assert!(
        sold.contains(mush_market::seller::BIDDER_NOT_BUYING),
        "expected BIDDER_NOT_BUYING, got: {sold}"
    );
}

/// **P2-MOD-30**: `trade::accept_at` re-checks `ProceedsPolicy::may_trade` so a buyer who took a
/// public quote on a restricted good (`strategic_ore` at Ceres) while holding valid papers cannot
/// accept that quote after incurring a Freeport trade restriction.
#[test]
fn p2_mod_30_accept_rechecks_trade_law_when_buyer_becomes_restricted() {
    let mut rig = stocked();
    let (seller, seller_row) = prospector(&mut rig, "LawSeller", 106, "Red Gull");
    let seller_hold = hold_vault_of(&rig.tw, &seller_row);
    let ore_lot = LotId(99_701);
    seed_lot(&mut rig, seller_hold, "strategic_ore", 2, ore_lot);
    mint_minor(&mut rig, seller, 10_000_000);
    crate::lw3_law_audit::seed_strategic_claim(&mut rig, seller);
    let _ = fire(&mut rig);

    let exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, seller, exchange);
    let posted = run(
        &mut rig.tw,
        seller,
        &format!("+market/post {ore_lot} price=30000"),
    );
    assert!(posted.contains("Listed for sale as #"), "{posted}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let listing_id = market(&rig)
        .live_listings()
        .find(|l| l.source.exact_lot() == Some(ore_lot))
        .map(|l| l.id)
        .expect("public strategic_ore listing");

    // Buyer starts licensed + holding a distinct strategic claim (rock #1), opens a locker, and
    // takes a remote quote.
    let (buyer, buyer_row) = prospector(&mut rig, "LawBuyer", 107, "Blue Finch");
    let buyer_hull = ObjId(buyer_row.hull);
    let buyer_hold = hold_vault_of(&rig.tw, &buyer_row);
    let (buyer_cluster, buyer_letter) = tacsim::space::mining::all_orbital_clusters(rig.tw.world())
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
        .nth(1)
        .expect("second strategic rock");
    let port_handles = rig
        .tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .cloned()
        .expect("port handles");
    let seed_buyer_claim = |rig: &mut Rig, active: bool| {
        let now = rig.tw.world().world_ic_now().at;
        let mut tenure =
            tacsim::surface::claims::BlockTenure::new(buyer_cluster.clone(), buyer_letter, buyer);
        if active {
            tenure.claim = Some(tacsim::surface::claims::BlockClaim {
                holder: PrincipalId::Object(buyer),
                account: rig
                    .tw
                    .world()
                    .module_state::<mush_sim::AccountOracle>()
                    .and_then(|oracle| oracle.account_of(buyer)),
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
        }
        crate::driving_rig::reseed_typed(
            rig,
            &port_handles.families.claims,
            &tacsim::surface::claims::BlockKey {
                site: buyer_cluster.clone(),
                letter: buyer_letter,
            },
            tenure,
        );
    };
    seed_buyer_claim(&mut rig, true);
    let _ = fire(&mut rig);
    let buyer_locker = open_locker_at(&mut rig, buyer, CERES, "room.c07_exchange", "ceres");
    let quoted = run(
        &mut rig.tw,
        buyer,
        &format!("+market/quote {listing_id}=2 remote"),
    );
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote_id = market(&rig)
        .quotes_of_buyer(&PrincipalId::Object(buyer))
        .map(|q| q.id)
        .max()
        .expect("quote");

    // 1. Remove `buyer`'s strategic claim so `buyer` loses papers while holding the open quote,
    // and verify `+market/accept` refuses on missing papers before creating any contract.
    seed_buyer_claim(&mut rig, false);
    let _ = fire(&mut rig);
    let (no_papers, queued) = command(
        &mut rig,
        buyer,
        &format!("+market/accept {quote_id}={}", buyer_locker.0),
    );
    assert!(
        queued.is_empty(),
        "must not stage without papers: {no_papers}"
    );
    assert!(
        no_papers.contains("registered claim on a strategic rock"),
        "expected missing-papers refusal at accept_at, got: {no_papers}"
    );
    assert!(
        market(&rig)
            .quote(quote_id)
            .is_some_and(|q| q.contract.is_none()),
        "no contract created when papers lapsed"
    );

    // 2. Drain `buyer`'s account, audit unlicensed strategic ore via real `+law/audit` to impose
    // a Freeport unpaid-fine restriction, then restore both papers + funds: `accept_at` must STILL
    // refuse because `buyer` is trade-restricted under Freeport!
    let treasury = port_map(&rig.tw, CERES).treasury.expect("treasury");
    let buyer_acct = tacsim::port::credit::own_account(rig.tw.world(), buyer)
        .expect("buyer account")
        .id;
    let avail = crate::lw3_law_audit::credit(&rig, buyer);
    if avail > 0 {
        let _ = run(
            &mut rig.tw,
            GOD,
            &format!("@econ/pay #{}=#{treasury} {avail}", buyer_acct.0),
        );
    }
    seed_pool(&mut rig, buyer_hold, "strategic_ore", 6, LotId(99_702));
    let audited = run(&mut rig.tw, GOD, &format!("+law/audit #{}", buyer_hull.0));
    assert!(audited.contains("recorded and fined"), "{audited}");
    let freeport = tacsim::polity::polity(rig.tw.world(), "freeport").expect("freeport");
    assert!(
        tacsim::port::record::restricted(rig.tw.world(), buyer, freeport),
        "buyer must now be trade-restricted under Freeport"
    );
    // Restore strategic claim and funds so ONLY the Freeport trade restriction blocks accept.
    seed_buyer_claim(&mut rig, true);
    let _ = fire(&mut rig);
    mint_minor(&mut rig, buyer, 5_000_000);

    let (accepted, queued) = command(
        &mut rig,
        buyer,
        &format!("+market/accept {quote_id}={}", buyer_locker.0),
    );
    assert!(queued.is_empty(), "must not stage draw/consume: {accepted}");
    assert!(
        accepted.contains("does not let you trade restricted goods"),
        "expected trade restriction refusal at accept_at, got: {accepted}"
    );
    assert!(
        market(&rig)
            .quote(quote_id)
            .is_some_and(|q| q.contract.is_none()),
        "no contract created when trade law refuses at accept"
    );
}

/// **P2-TSW-9**: `+law/report` requires the reporter to stand at `r.venue`, requires a reporting
/// buyer to still control `VaultId(r.destination)` while the vault/hull is docked and not cleared
/// at `r.venue`, and guards projected offers with a `RevisionValidator` against TOCTOU transfers.
#[test]
#[allow(clippy::too_many_lines)]
fn p2_tsw_9_law_report_requires_presence_live_vault_control_and_docked_hull() {
    let mut rig = stocked();
    let (seller, seller_row) = prospector(&mut rig, "PrivSmuggler", 108, "Grey Skua");
    let seller_hold = hold_vault_of(&rig.tw, &seller_row);
    let ore_lot = LotId(99_801);
    seed_lot(&mut rig, seller_hold, "strategic_ore", 2, ore_lot);
    mint_minor(&mut rig, seller, 10_000_000);

    let (buyer, buyer_row) = prospector(&mut rig, "PrivBuyer", 109, "Salt Kite");
    let buyer_hull = ObjId(buyer_row.hull);
    let buyer_hold = hold_vault_of(&rig.tw, &buyer_row);
    mint_minor(&mut rig, buyer, 10_000_000);

    // Private sale from unlicensed `seller` directly into `buyer`'s hull hold at Ceres.
    let ceres_exchange = station_room(&rig.tw, CERES, "room.c07_exchange");
    teleport(&mut rig.tw, seller, ceres_exchange);
    teleport(&mut rig.tw, buyer, ceres_exchange);

    let offered = run(
        &mut rig.tw,
        seller,
        &format!("+market/post {ore_lot} price=30000 to=PrivBuyer"),
    );
    assert!(offered.contains("Listed for sale as #"), "{offered}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let listing_id = market(&rig)
        .live_listings()
        .find(|l| l.source.exact_lot() == Some(ore_lot))
        .map(|l| l.id)
        .expect("private listing");

    let quoted = run(&mut rig.tw, buyer, &format!("+market/quote {listing_id}=2"));
    assert!(quoted.contains("Quote #"), "{quoted}");
    let quote = market(&rig)
        .quotes_of_buyer(&PrincipalId::Object(buyer))
        .map(|q| q.id.0)
        .max()
        .expect("quote");
    let accepted = run(
        &mut rig.tw,
        buyer,
        &format!("+market/accept {quote}={}", buyer_hold.0),
    );
    let queued = drain(&mut rig.tw);
    assert!(!queued.is_empty(), "{accepted}");
    land_all(&mut rig, &queued);
    for _ in 0..3 {
        crate::driving_rig::sweep(&mut rig.tw, 60);
        settle(&mut rig, 1);
    }

    let receipt = tacsim::port::law_report::receipts_of(rig.tw.world(), buyer)
        .and_then(|b| b.receipts.into_iter().find(|r| r.quote == quote))
        .expect("buyer receipt");
    assert_eq!(receipt.destination, buyer_hold.0);
    assert!(!receipt.seller_papers);

    // 1. Remote reporter standing at Stickney is refused with `REPORT_NOT_HERE`.
    let stickney_exchange = station_room(&rig.tw, STICKNEY, "room.p08_exchange");
    teleport(&mut rig.tw, buyer, stickney_exchange);
    let (out, queued) = command(&mut rig, buyer, &format!("+law/report {quote} go"));
    assert!(queued.is_empty(), "remote report must not queue: {out}");
    assert!(
        out.contains(tacsim::port::law_report::REPORT_NOT_HERE),
        "expected REPORT_NOT_HERE, got: {out}"
    );

    // 2. Cleared-to-sail hull at Ceres refuses `+law/report` with `HOLD_CLEARED`.
    seed_hold(&mut rig, &buyer_row, "propellant", 80, 99_802);
    seed_hold(&mut rig, &buyer_row, "provisions", 100, 99_803);
    let ceres_gallery = station_room(&rig.tw, CERES, "room.c01_berth_gallery");
    teleport(&mut rig.tw, buyer, ceres_gallery);
    board(&mut rig, buyer);
    let undock = run(&mut rig.tw, buyer, "+ship/undock stickney");
    let burn_queued = drain(&mut rig.tw);
    assert!(
        tacsim::port::voyage::pending_of(rig.tw.world(), buyer_hull).is_some(),
        "clearance filed: {undock}"
    );
    land_all(&mut rig, &burn_queued);
    let unplaced = run(&mut rig.tw, GOD, &format!("@spatial/unplace #{}", buyer.0));
    assert!(unplaced.contains("Unplaced"), "{unplaced}");
    teleport(&mut rig.tw, buyer, ceres_exchange);
    let (out, queued) = command(&mut rig, buyer, &format!("+law/report {quote} go"));
    assert!(
        queued.is_empty(),
        "cleared hull report must not queue: {out}"
    );
    assert!(
        out.contains(
            &tacsim::port::market::HOLD_CLEARED
                .replace("{name}", "Salt Kite")
                .replace("{port}", "Ceres Freeport")
        ),
        "expected HOLD_CLEARED, got: {out}"
    );

    // 3. Under-way hull refuses `+law/report` (no remote in-flight teleportation!) and refuses
    // `PortVenueGate::receives_at` with `HOLD_UNDER_WAY`.
    let airlock = buyer_row.rooms[&cid("airlock")];
    let placed = run(
        &mut rig.tw,
        GOD,
        &format!("@spatial/place/interior #{}=#{airlock},berths", buyer.0),
    );
    assert!(!placed.contains("Usage"), "{placed}");
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert!(
        matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), buyer_hull),
            tacsim::port::ship::Berth::UnderWay
        ),
        "Salt Kite must be under way"
    );
    let (out, queued) = command(&mut rig, buyer, &format!("+law/report {quote} go"));
    assert!(queued.is_empty(), "under-way report must not queue: {out}");
    assert!(
        out.contains(tacsim::port::law_report::REPORT_NOT_HERE),
        "expected REPORT_NOT_HERE while under way, got: {out}"
    );
    let ceres_id = mush_market::MarketId::new("ceres").expect("ceres");
    let under_way_err = mush_market::VenueGate::receives_at(
        &tacsim::port::market::PortVenueGate,
        rig.tw.world(),
        &ceres_id,
        EndpointRef(buyer_hold),
    )
    .expect_err("under-way hull hold must refuse receives_at");
    assert_eq!(
        under_way_err.line,
        tacsim::port::market::HOLD_UNDER_WAY
            .replace("{name}", "Salt Kite")
            .replace("{port}", "Ceres Freeport")
    );

    // 4. Post-projection TOCTOU revision guard + cross-owner refusal on a fresh docked sale.
    let ore_lot2 = LotId(99_811);
    seed_lot(&mut rig, seller_hold, "strategic_ore", 2, ore_lot2);
    let (buyer2, buyer2_row) = prospector(&mut rig, "PrivBuyer2", 110, "Sea Wren");
    let buyer2_hold = hold_vault_of(&rig.tw, &buyer2_row);
    mint_minor(&mut rig, buyer2, 10_000_000);
    teleport(&mut rig.tw, seller, ceres_exchange);
    teleport(&mut rig.tw, buyer2, ceres_exchange);
    let offered2 = run(
        &mut rig.tw,
        seller,
        &format!("+market/post {ore_lot2} price=30000 to=PrivBuyer2"),
    );
    assert!(offered2.contains("Listed for sale as #"), "{offered2}");
    let queued = drain(&mut rig.tw);
    land_all(&mut rig, &queued);
    let listing2 = market(&rig)
        .live_listings()
        .find(|l| l.source.exact_lot() == Some(ore_lot2))
        .map(|l| l.id)
        .expect("second private listing");
    let quoted2 = run(&mut rig.tw, buyer2, &format!("+market/quote {listing2}=2"));
    assert!(quoted2.contains("Quote #"), "{quoted2}");
    let quote2 = market(&rig)
        .quotes_of_buyer(&PrincipalId::Object(buyer2))
        .map(|q| q.id.0)
        .max()
        .expect("second quote");
    let accepted2 = run(
        &mut rig.tw,
        buyer2,
        &format!("+market/accept {quote2}={}", buyer2_hold.0),
    );
    let queued = drain(&mut rig.tw);
    assert!(!queued.is_empty(), "{accepted2}");
    land_all(&mut rig, &queued);
    for _ in 0..3 {
        crate::driving_rig::sweep(&mut rig.tw, 60);
        settle(&mut rig, 1);
    }

    // Queue `+law/report` while buyer2 still controls `buyer2_hold`, and project its offer +
    // revision guards before transferring the hold to `victim`.
    let (out, queued) = command(&mut rig, buyer2, &format!("+law/report {quote2} go"));
    assert_eq!(queued.len(), 1, "valid docked report queues: {out}");
    let guards = {
        let engine = rig.tw.engine();
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: buyer2 },
                buyer2,
                mush_core::PerceivedRef::SelfActor,
            )
            .expect("query");
        let offers = engine.project_legal_actions(&query).expect("offers");
        let projected = offers
            .projected_for_action(&queued[0].action)
            .expect("law_report offer must project while buyer2 owns docked hold");
        let token = [0x5au8; 16];
        let submission = mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters: queued[0].parameters.clone(),
        };
        let execution = mush_core::OfferExecutionContext {
            request: mush_core::RequestUid::from_bytes(token),
            principal: mush_core::PrincipalId::Object { id: buyer2 },
            actor: buyer2,
            idempotency: mush_core::ScopedIdempotencyKey::new(
                mush_core::IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: buyer2 },
                    owner: queued[0].action.key().owner.clone(),
                    namespace: cid(queued[0].action.key().id.as_str()),
                },
                1,
                mush_core::IdempotencyKey::Request(mush_core::RequestUid::from_bytes(token)),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .expect("resolve offer");
        let (_, _, guards) = resolved.into_authoritative_parts();
        assert!(
            engine
                .revalidate_guards(&guards)
                .is_ok_and(|res| res.is_current()),
            "guards must be current before re-home"
        );
        guards
    };

    let victim = pilot(&mut rig, "InnocentBuyer", 111);
    {
        let engine = rig.tw.engine_mut();
        let mut outbox = mush_core::Outbox::new();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: Default::default(),
            args: Default::default(),
            out: &mut outbox,
            reg: &engine.registry,
            module: Some(tacsim::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        assert!(mush_economy::rehome_vault(
            &mut ctx,
            buyer2_hold,
            PrincipalId::Object(victim),
            None,
            smol_str::SmolStr::new("Sea Wren hold"),
        ));
    }

    // 4a. Pre-projected offer guards are immediately invalidated by `LawReportValidator`.
    assert!(
        !rig.tw
            .engine()
            .revalidate_guards(&guards)
            .is_ok_and(|res| res.is_current()),
        "revision validator must invalidate pre-projected report offer after vault re-home"
    );

    // 4b. Attempting to land the queued report after vault re-home must NOT seize the lot.
    land_quietly(&mut rig, &queued);
    let families = rig
        .tw
        .world()
        .module_state::<mush_inventory::InventoryHandles>()
        .expect("inventory")
        .0
        .clone();
    assert_eq!(
        typed(&mut rig.tw, &families.lots, &ore_lot2).map(|l| l.endpoint()),
        Some(EndpointRef(buyer2_hold)),
        "victim's hold must NOT be seized after re-home TOCTOU"
    );

    // 4c. Direct `+law/report` after transfer/re-home refuses upfront with `REPORT_NO_GOODS`.
    let (out, queued) = command(&mut rig, buyer2, &format!("+law/report {quote2} go"));
    assert!(
        queued.is_empty(),
        "cross-owner report must not queue: {out}"
    );
    assert!(
        out.contains(tacsim::port::law_report::REPORT_NO_GOODS),
        "expected REPORT_NO_GOODS, got: {out}"
    );
}
