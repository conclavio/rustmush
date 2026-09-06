//! E7 cut 4's verification, by scenario, over the three lenses 10b §3.4 names:
//! **A** atomicity and contention, **G** generation, **P** projection.
//!
//! The rig is `crates/mush-industry/tests/e7c3.rs`'s shape: `boot()` composes the
//! economy, cut 2 and cut 3/4 in production order, `apply()` opens the same
//! owner-stamped transaction a provider gets and **installs** it exactly as a
//! receipt does, so every "writes nothing" assertion means what it says.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::{
    CanonicalId, ClockRef, ClockStamp, CommitToken, DefinitionDigest, Engine, GOD, IdempotencyKey,
    IdempotencyScope, ObjId, OwnerId, RequestUid, ScopedIdempotencyKey,
};
use mush_economy::{AccountId, EconomicVisibility, VaultId};
use mush_industry::facility::{FacilityRecord, MAX_FACILITY_SLOTS};
use mush_industry::record::{MAX_RETAINED_MARKERS_PER_DEPOSIT, MarkerRing};
use mush_industry::workorder::{
    DeliveryTarget, MAX_WORK_ORDER_INPUTS, PinnedRecipe, ProductionSchedule, RecipeLine, RecipeRef,
    WorkOrder, WorkOrderId, WorkOrderMarker, WorkOrderState, quote_duration, quote_price,
    wo_holder, wo_holder_text,
};
use mush_industry::{
    Accessibility, BandId, CurvePoint, ExtractionRun, OreDeposit, OreDepositId, YieldCurve,
};
use mush_industry::{IndustryError, IndustryFamilies, IndustryHandles, IndustryModule};
use mush_inventory::{EndpointRef, Grade, InventoryHandleCell, InventoryModule};
use mush_org::PrincipalId;
use mush_testkit::TestWorld;

// ---- E7c4: the rig ----

fn boot() -> (TestWorld, IndustryFamilies) {
    let cell: InventoryHandleCell = Arc::default();
    let world = TestWorld::with_modules(vec![
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["industry"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(IndustryModule::new().with_inventory(Arc::clone(&cell))),
    ]);
    let families = world
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry publishes its handles")
        .0
        .clone();
    (world, families)
}

fn now() -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 1_000,
    }
}

fn holder(actor: ObjId, tag: &str, byte: u8) -> ScopedIdempotencyKey {
    ScopedIdempotencyKey::new(
        IdempotencyScope {
            principal: mush_core::PrincipalId::Object { id: actor },
            owner: OwnerId::new("industry").expect("owner"),
            namespace: CanonicalId::new(tag).expect("namespace"),
        },
        0,
        IdempotencyKey::Request(RequestUid::from_bytes([byte; 16])),
    )
}

fn apply<T>(
    engine: &mut Engine,
    families: &IndustryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, IndustryError>,
) -> Result<T, IndustryError> {
    let (result, prepared) = {
        let mut txn = engine
            .action_transaction(&families.deposits)
            .expect("industry transaction");
        let result = body(&mut txn);
        match result {
            Ok(value) => (Ok(value), txn.prepare().ok()),
            Err(error) => (Err(error), None),
        }
    };
    if let Some(prepared) = prepared {
        let (_, installation) = prepared.into_parts();
        engine.install_prepared_action(installation);
    }
    result
}

fn read<T>(
    engine: &mut Engine,
    families: &IndustryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> T,
) -> T {
    let mut txn = engine
        .action_transaction(&families.deposits)
        .expect("industry transaction");
    body(&mut txn)
}

fn capabilities(ids: &[&str]) -> BTreeSet<CanonicalId> {
    ids.iter()
        .map(|id| CanonicalId::new(id).expect("capability id"))
        .collect()
}

fn commodity(id: &str) -> CanonicalId {
    CanonicalId::new(id).expect("commodity id")
}

fn endpoint(id: u64) -> EndpointRef {
    EndpointRef(VaultId(id))
}

/// The `alloy_smelt` pinned recipe: 4 metals -> 1 structural_alloys, 3 work units.
fn alloy_recipe() -> (RecipeRef, PinnedRecipe) {
    let recipe = RecipeRef::new(
        commodity("tacsim.core"),
        1,
        commodity("alloy_smelt"),
        DefinitionDigest::from_bytes([0x11; 32]),
    )
    .expect("recipe ref");
    let pinned = PinnedRecipe::new(
        vec![RecipeLine::new(commodity("metals"), 4).expect("input line")],
        vec![RecipeLine::new(commodity("structural_alloys"), 1).expect("output line")],
        capabilities(&["smelt"]),
        3,
    )
    .expect("pinned recipe");
    (recipe, pinned)
}

fn draft_order(id: u64, facility: ObjId, quantity: u32, source: u64, dest: u64) -> WorkOrder {
    let (recipe, pinned) = alloy_recipe();
    WorkOrder::draft(
        WorkOrderId(id),
        PrincipalId::Object(GOD),
        PrincipalId::Object(GOD),
        facility,
        2,
        1,
        1,
        recipe,
        pinned,
        quantity,
        AccountId(1),
        Some(endpoint(source)),
        DeliveryTarget::Endpoint {
            endpoint: endpoint(dest),
        },
        EconomicVisibility::Parties,
    )
    .expect("draft a work order")
}

// ---- E7c4: B1 — facilities, the capacity book, the second Reservation validator ----

#[test]
fn e7c4_b1_the_facility_record_carries_no_active_slots_counter() {
    // OPEN 1: a stored occupancy counter is the fail-open shape. The record's
    // serialized field set is exactly the definition/policy half.
    let facility = FacilityRecord::new(
        ObjId(7),
        PrincipalId::Object(GOD),
        capabilities(&["forge"]),
        4,
    )
    .expect("author a facility");
    let value = serde_json::to_value(&facility).expect("serialize");
    let object = value.as_object().expect("facility is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "capabilities",
            "capacity_revision",
            "maximum_slots",
            "object",
            "owner",
            "policy_revision"
        ],
        "a facility record has no active_slots (or any occupancy) field"
    );
}

#[test]
fn e7c4_b1_availability_is_derived_from_live_reservations() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let facility = ObjId(7);
    // No book yet: every slot free, derived, with no counter to read.
    let free = read(engine, &families, |txn| {
        families.available_slots(txn, facility, 3)
    })
    .expect("read availability");
    assert_eq!(free, 3, "a facility with no book has every slot free");

    // Hold one slot; availability falls to 2, derived from the live hold.
    apply(engine, &families, |txn| {
        families.reserve_slot(txn, facility, 3, &holder(GOD, "wo", 1), &now())?;
        Ok(())
    })
    .expect("reserve a slot");
    let free = read(engine, &families, |txn| {
        families.available_slots(txn, facility, 3)
    })
    .expect("read availability");
    assert_eq!(free, 2, "one held slot lowers derived availability by one");
}

#[test]
fn e7c4_b1_two_orders_contend_for_the_last_slot_and_exactly_one_starts() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let facility = ObjId(9);
    // maximum_slots == 1: the last slot.
    apply(engine, &families, |txn| {
        families.reserve_slot(txn, facility, 1, &holder(GOD, "wo", 1), &now())?;
        Ok(())
    })
    .expect("the first order takes the last slot");

    // The second holder contends for the same one slot and refuses at K's
    // `checked_hold` (Availability), never at a cut-4 check.
    let second = apply(engine, &families, |txn| {
        families.reserve_slot(txn, facility, 1, &holder(ObjId(2), "wo", 2), &now())?;
        Ok(())
    });
    assert!(
        matches!(second, Err(IndustryError::Book(_))),
        "the second order refuses at the kernel's capacity gate: {second:?}"
    );
    let free = read(engine, &families, |txn| {
        families.available_slots(txn, facility, 1)
    })
    .expect("read availability");
    assert_eq!(free, 0, "the one slot is held");
}

#[test]
fn e7c4_b1_a_released_slot_is_reservable_again() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let facility = ObjId(11);
    let id = apply(engine, &families, |txn| {
        families.reserve_slot(txn, facility, 1, &holder(GOD, "wo", 1), &now())
    })
    .expect("reserve");
    // Release, then the same holder reserves again (a completed order frees its
    // slot for the next).
    apply(engine, &families, |txn| {
        families.release_slot(txn, facility, id, 1)
    })
    .expect("release");
    let free = read(engine, &families, |txn| {
        families.available_slots(txn, facility, 1)
    })
    .expect("availability");
    assert_eq!(free, 1, "the released slot is free again");
    apply(engine, &families, |txn| {
        families.reserve_slot(txn, facility, 1, &holder(GOD, "wo", 1), &now())?;
        Ok(())
    })
    .expect("re-reserve after release retires the terminal record first");
}

#[test]
fn e7c4_b1_both_reservation_validators_compose_without_collision() {
    // Booting composes cut 2's inventory `Reservation` validator AND cut 4's
    // capacity `Reservation` validator, each under its own `OwnerId`. The
    // registry is keyed `(OwnerId, kind)`, so the composition does not collide —
    // if it did, `boot()` would panic. A held capacity slot proves cut 4's
    // validator is the live one for industry-owned guards.
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let id = apply(engine, &families, |txn| {
        families.reserve_slot(
            txn,
            ObjId(13),
            MAX_FACILITY_SLOTS,
            &holder(GOD, "wo", 1),
            &now(),
        )
    })
    .expect("a capacity hold under industry's own Reservation validator");
    let record = read(engine, &families, |txn| {
        families.slot_reservation(txn, ObjId(13), id)
    })
    .expect("read")
    .expect("the hold exists");
    assert_eq!(record.amount(), 1, "a capacity hold is one slot");
}

// ---- E7c4: B3 — the work-order record and state machine ----

#[test]
fn e7c4_b3_the_state_machine_transitions_bump_revision() {
    let order = draft_order(0xA1, ObjId(7), 5, 500, 600);
    assert_eq!(order.revision(), 1);
    assert_eq!(*order.state(), WorkOrderState::Draft);
    let running = order
        .acquired(Vec::new(), Vec::new(), None, WorkOrderState::Running)
        .expect("acquire");
    assert_eq!(running.revision(), 2);
    assert_eq!(*running.state(), WorkOrderState::Running);
    let done = running
        .with_state(WorkOrderState::Completed {
            commit: CommitToken::from_bytes([9; 16]),
        })
        .expect("complete");
    assert_eq!(done.revision(), 3);
    assert!(done.state().is_terminal());
}

#[test]
fn e7c4_b3_work_order_wire_round_trips_and_refuses_over_cap() {
    let order = draft_order(0xB2, ObjId(7), 2, 500, 600);
    let value = serde_json::to_value(&order).expect("serialize");
    let back: WorkOrder = serde_json::from_value(value).expect("round trip");
    assert_eq!(order, back);
    // MAX_WORK_ORDER_INPUTS is enforced at construction, refused not truncated.
    let over: Vec<RecipeLine> = (0..=MAX_WORK_ORDER_INPUTS)
        .map(|i| RecipeLine::new(commodity(&format!("c{i}")), 1).expect("line"))
        .collect();
    assert!(
        PinnedRecipe::new(
            over,
            vec![RecipeLine::new(commodity("out"), 1).unwrap()],
            BTreeSet::new(),
            1
        )
        .is_err(),
        "a pinned recipe over MAX_WORK_ORDER_INPUTS refuses on construction"
    );
}

#[test]
fn e7c4_b3_the_deterministic_holder_key_fits_the_idempotency_cap() {
    // `wo/{order}` is inside MAX_IDEMPOTENCY_KEY_BYTES (128), and the derived
    // holder is stable per work order (so the terminal transition re-derives the
    // reservations submit placed).
    let text = wo_holder_text(WorkOrderId(0xC3));
    assert!(text.len() <= 128, "holder text fits the idempotency cap");
    let a = wo_holder(WorkOrderId(0xC3)).expect("holder");
    let b = wo_holder(WorkOrderId(0xC3)).expect("holder");
    assert_eq!(a, b, "the holder key is deterministic per work order");
    assert_ne!(
        a,
        wo_holder(WorkOrderId(0xC4)).expect("holder"),
        "different orders derive different holders"
    );
}

// ---- E7c4: A — conservation across a run's material inputs ----

#[test]
fn e7c4_a_a_work_order_run_conserves_its_inputs() {
    // Reserve -> consume -> mint, exactly as wo_submit + wo_deliver do through the
    // families, and prove the source pool falls by 4 and the output lot holds 1 —
    // cut 4 writing no arithmetic; every debit is cut 2's over checked_apply.
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let source = endpoint(500);
    let dest = endpoint(600);
    let metals = commodity("metals");
    let alloys = commodity("structural_alloys");
    apply(engine, &families, |txn| {
        families
            .inventory
            .credit(txn, source, &metals, 20)
            .map(|_| ())
            .map_err(IndustryError::Inventory)
    })
    .expect("seed metals");

    let holder = wo_holder(WorkOrderId(0xD1)).expect("holder");
    let commit = CommitToken::from_bytes([0xD1; 16]);
    // Reserve 4 metals (quantity 1 x 4 units).
    let rid = apply(engine, &families, |txn| {
        families
            .inventory
            .reserve(txn, source, &metals, &holder, 4, &now())
            .map_err(IndustryError::Inventory)
    })
    .expect("reserve materials");
    // Consume the reservation and mint the output.
    apply(engine, &families, |txn| {
        let record = families
            .inventory
            .reservation(txn, source, &metals, rid)
            .map_err(IndustryError::Inventory)?
            .expect("the hold exists");
        families
            .inventory
            .consume(txn, source, &metals, rid, record.revision(), &now(), commit)
            .map_err(IndustryError::Inventory)?;
        let lot = mush_inventory::derive_lot_id(RequestUid::from_bytes([0xD1; 16]), 0)
            .map_err(IndustryError::Inventory)?;
        families
            .inventory
            .mint_origin(
                txn,
                dest,
                &alloys,
                1,
                Grade::new(0).unwrap(),
                mush_economy::ExternalRef {
                    provider: smol_str::SmolStr::new_static("industry"),
                    kind: smol_str::SmolStr::new_static("workorder.output"),
                    id: smol_str::SmolStr::new("wo/000000000000d1"),
                    revision: Some(1),
                },
                lot,
            )
            .map(|_| ())
            .map_err(IndustryError::Inventory)
    })
    .expect("consume and mint");

    // The source pool fell by exactly 4; conservation holds through cut 2's gate.
    let remaining = read(engine, &families, |txn| {
        families.inventory.available(txn, source, &metals)
    })
    .expect("available");
    assert_eq!(remaining, 16, "4 metals consumed, 16 remain");
    // The output is a minted Lot at the destination (not a pool credit): exactly
    // one lot of one unit.
    let lot_id = mush_inventory::derive_lot_id(RequestUid::from_bytes([0xD1; 16]), 0).unwrap();
    let quantity = read(engine, &families, |txn| {
        txn.get(&families.inventory.lots, &lot_id)
            .expect("read lot")
            .map(mush_inventory::Lot::quantity)
    });
    assert_eq!(
        quantity,
        Some(1),
        "exactly one structural_alloys lot minted"
    );
    let _ = alloys;
}

#[test]
fn e7c4_a_cancel_releases_the_material_reservation() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let source = endpoint(700);
    let metals = commodity("metals");
    apply(engine, &families, |txn| {
        families
            .inventory
            .credit(txn, source, &metals, 10)
            .map(|_| ())
            .map_err(IndustryError::Inventory)
    })
    .expect("seed");
    let holder = wo_holder(WorkOrderId(0xE1)).expect("holder");
    let rid = apply(engine, &families, |txn| {
        families
            .inventory
            .reserve(txn, source, &metals, &holder, 4, &now())
            .map_err(IndustryError::Inventory)
    })
    .expect("reserve");
    let held = read(engine, &families, |txn| {
        families.inventory.available(txn, source, &metals)
    })
    .expect("available");
    assert_eq!(held, 6, "4 of 10 held");
    // Cancel releases it — availability restored, nothing consumed.
    apply(engine, &families, |txn| {
        families
            .inventory
            .release(txn, source, &metals, rid, 1)
            .map_err(IndustryError::Inventory)
    })
    .expect("release");
    let freed = read(engine, &families, |txn| {
        families.inventory.available(txn, source, &metals)
    })
    .expect("available");
    assert_eq!(freed, 10, "release restores full availability");
}

// ---- E7c4: G — generation and progress ----

#[test]
fn e7c4_g_production_progress_is_bounded_by_total_units() {
    let schedule = mush_core::ScheduleId::derive(
        &OwnerId::new("industry").unwrap(),
        &commodity("production.test"),
    )
    .expect("schedule id");
    let production =
        ProductionSchedule::new(schedule, now(), commodity("production"), 3, 0).expect("schedule");
    assert!(!production.is_complete());
    let one = production.advanced(1).expect("advance");
    assert_eq!(one.progress_units(), 1);
    // An over-advance clamps to total_units, never exceeds it.
    let done = one.advanced(100).expect("advance clamps");
    assert_eq!(
        done.progress_units(),
        3,
        "progress is bounded by total_units"
    );
    assert!(done.is_complete());
}

// ---- E7c4: P — the quote is a pure function of recorded inputs ----

#[test]
fn e7c4_p_a_quote_is_a_pure_function_of_recorded_inputs() {
    let (_recipe, pinned) = alloy_recipe();
    // Price is the total input units the run consumes; duration is work_units x qty.
    assert_eq!(quote_price(&pinned, 5), 20, "4 metals x 5 = 20 input units");
    assert_eq!(quote_duration(&pinned, 5), 15, "3 work units x 5 = 15");
    // Deterministic: two evaluations agree, and it reads no world/holdings.
    assert_eq!(quote_price(&pinned, 5), quote_price(&pinned, 5));
}

// ---- E7c4: B7 — the completion marker answers the work-order fact ----

#[test]
fn e7c4_b7_the_completion_marker_round_trips_and_names_its_fact() {
    let marker = WorkOrderMarker::new(
        "industry.workorder.completed",
        WorkOrderId(0xF1),
        GOD,
        1_000,
    )
    .expect("marker");
    assert_eq!(marker.kind(), "industry.workorder.completed");
    let value = serde_json::to_value(&marker).expect("serialize");
    let back: WorkOrderMarker = serde_json::from_value(value).expect("round trip");
    assert_eq!(marker, back);
    // A marker naming any other fact is refused.
    assert!(WorkOrderMarker::new("industry.extraction.cycled", WorkOrderId(1), GOD, 1).is_err());
}

// ---- E7c4: D2 — the durable marker family is bounded per deposit ----

#[test]
fn e7c4_d2_the_marker_ring_evicts_the_oldest_past_the_cap() {
    let mut ring = MarkerRing::empty();
    let mut last_evicted = None;
    for i in 0..(MAX_RETAINED_MARKERS_PER_DEPOSIT as u32 + 3) {
        let (next, evicted) = ring.push(smol_str::SmolStr::new(format!("k{i:04}")));
        ring = next;
        if evicted.is_some() {
            last_evicted = evicted;
        }
    }
    assert_eq!(
        ring.len(),
        MAX_RETAINED_MARKERS_PER_DEPOSIT,
        "the ring never exceeds the retention cap"
    );
    assert_eq!(
        last_evicted.as_deref(),
        Some("k0002"),
        "eviction is FIFO — the oldest key leaves first"
    );
}

#[test]
fn e7c4_d2_the_durable_marker_family_stays_bounded_and_conserves() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    // A tiny body: reserve 12, yield 1/cycle — 12 cycles, past the retention of 8.
    let commodity = commodity("metals");
    let body = OreDeposit::new(
        OreDepositId(0xD2),
        ObjId(42),
        commodity.clone(),
        Grade::new(50).unwrap(),
        Accessibility::new(100).unwrap(),
        BandId::new("grade_b").unwrap(),
        BandId::new("reserve_minor").unwrap(),
        YieldCurve::new(vec![CurvePoint {
            cumulative: 0,
            per_cycle: 1,
        }])
        .unwrap(),
        endpoint(800),
        12,
    )
    .unwrap();
    apply(engine, &families, |txn| {
        families.open_deposit(txn, body.clone())
    })
    .expect("open");
    apply(engine, &families, |txn| {
        families.put_run(
            txn,
            ExtractionRun::new(body.id(), PrincipalId::Object(GOD), endpoint(801)).unwrap(),
        )
    })
    .expect("run");
    for i in 0..12u32 {
        let request = RequestUid::from_bytes([(i + 1) as u8; 16]);
        let commit = CommitToken::from_bytes([(i + 1) as u8; 16]);
        apply(engine, &families, |txn| {
            let live = families.deposit(txn, body.id())?.expect("body");
            let run = families.run(txn, body.id())?.expect("run");
            families.run_cycle(
                txn,
                &live,
                &run,
                GOD,
                request,
                &holder(GOD, "extract", (i + 1) as u8),
                &now(),
                commit,
                mush_economy::ExternalRef {
                    provider: smol_str::SmolStr::new_static("industry"),
                    kind: smol_str::SmolStr::new_static("extraction.cycle"),
                    id: smol_str::SmolStr::new(format!("extract/x/{i}")),
                    revision: Some(1),
                },
            )?;
            Ok(())
        })
        .expect("cycle");
    }
    // The durable applied family is bounded per deposit — old markers were pruned.
    let markers = engine
        .action_state()
        .scan_family(&families.applied, None, 64)
        .expect("scan applied")
        .entries()
        .len();
    assert!(
        markers <= MAX_RETAINED_MARKERS_PER_DEPOSIT,
        "the durable marker family is bounded per deposit ({markers} retained)"
    );
    // Conservation held regardless of pruning: the pool is exhausted (12 mined).
    let remaining = read(engine, &families, |txn| {
        families.inventory.available(txn, endpoint(800), &commodity)
    })
    .expect("available");
    assert_eq!(
        remaining, 0,
        "all 12 units extracted; pruning touched no quantity"
    );
}
