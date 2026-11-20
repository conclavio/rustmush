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
