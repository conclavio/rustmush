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
    CanonicalId, ClockRef, ClockStamp, CommitToken, DefinitionDigest, DurableSchedule,
    DurableScheduleDraft, Engine, GOD, IdempotencyKey, IdempotencyScope, ObjId, OwnerId,
    RequestUid, ScheduleCatchUpOrder, ScheduleDriveCause, ScheduleId, ScheduleRestartPolicy,
    ScopedIdempotencyKey, World,
};
use mush_economy::{AccountId, EconomicVisibility, VaultId};
use mush_industry::facility::{FacilityRecord, MAX_FACILITY_SLOTS, facility_capacity_key};
use mush_industry::record::{MAX_RETAINED_MARKERS_PER_DEPOSIT, MarkerRing};
use mush_industry::workorder::{
    DeliveryTarget, MAX_WORK_ORDER_INPUTS, PRODUCTION_CADENCE_SECS, PRODUCTION_MAX_CATCH_UP,
    PRODUCTION_UNITS_PER_OCCURRENCE, PinnedRecipe, ProductionJob, ProductionSchedule, RecipeLine,
    RecipeRef, WorkOrder, WorkOrderId, WorkOrderMarker, WorkOrderState, quote_duration,
    quote_price, wo_holder, wo_holder_text,
};
use mush_industry::{
    Accessibility, BandId, CurvePoint, ExtractionRun, OreDeposit, OreDepositId, YieldCurve,
};
use mush_industry::{
    FacilityQuarantine, IndustryError, IndustryFamilies, IndustryHandles, IndustryModule,
    IndustryStore,
};
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
        engine
            .install_prepared_action(installation)
            .expect("the fixture installs");
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

// ============================================================================
// ---- E7 cut 4 independent verify fold ----
// The twelve confirmed findings, one regression each. Names `vf_<lens><n>_...`.
// Owner decisions taken: D1 (bind at command + refuse zero-input + plan digest
// drift, no kernel prepare-seam), D2 (owner/staff facility auth + record-sourced
// capacity + per-requester cap), D3(a) (bound wo_applied via a per-facility ring),
// D4(a) (kernel scan skips a stale generation + @wo/restamp reconciles), R1/R4
// mandatory.
// ============================================================================

/// A test recipe authority: it resolves exactly the corridor's `alloy_smelt` and
/// answers its digest, so `@wo/draft` binds a real recipe and the command-level
/// gates can be exercised. Anything else is unauthored (fails closed).
///
/// WS02-S4 gave it two more jobs: it **counts** its catalog walks, so the draft
/// offer's cache is observable, and it can author `filler` extra runnable recipes,
/// so the offer's recipe-book cap is reachable.
#[derive(Debug, Default)]
struct TestRecipes {
    catalogs: Arc<std::sync::atomic::AtomicUsize>,
    filler: usize,
}

/// The pack-local id of one filler recipe.
fn filler_id(index: usize) -> String {
    format!("filler_{index:04}")
}

/// One filler recipe: a distinct input commodity, the corridor's own capability.
fn filler_recipe(index: usize) -> (RecipeRef, PinnedRecipe) {
    let recipe = RecipeRef::new(
        commodity("tacsim.core"),
        1,
        commodity(&filler_id(index)),
        DefinitionDigest::from_bytes([0x11; 32]),
    )
    .expect("filler recipe ref");
    let pinned = PinnedRecipe::new(
        vec![RecipeLine::new(commodity("metals"), 4).expect("input line")],
        vec![
            RecipeLine::new(
                commodity("structural_alloys"),
                u64::try_from(index % 7 + 1).unwrap(),
            )
            .expect("output line"),
        ],
        capabilities(&["smelt"]),
        3,
    )
    .expect("filler pinned");
    (recipe, pinned)
}

impl TestRecipes {
    fn with_filler(filler: usize) -> Self {
        Self {
            catalogs: Arc::default(),
            filler,
        }
    }
}

impl mush_industry::RecipeAuthority for TestRecipes {
    fn digest_of(&self, _world: &World, recipe: &RecipeRef) -> Option<DefinitionDigest> {
        (recipe.pack().as_str() == "tacsim.core"
            && recipe.version() == 1
            && (recipe.id().as_str() == "alloy_smelt"
                || recipe.id().as_str().starts_with("filler_")))
        .then(|| DefinitionDigest::from_bytes([0x11; 32]))
    }
    fn resolve(
        &self,
        _world: &World,
        pack: &CanonicalId,
        version: u32,
        id: &CanonicalId,
    ) -> Option<(RecipeRef, PinnedRecipe)> {
        if pack.as_str() != "tacsim.core" || version != 1 {
            return None;
        }
        if id.as_str() == "alloy_smelt" {
            return Some(alloy_recipe());
        }
        (0..self.filler)
            .find(|index| filler_id(*index) == id.as_str())
            .map(filler_recipe)
    }
    fn catalog_revision(&self, _world: &World) -> u64 {
        1
    }
    fn catalog(&self, _world: &World) -> Vec<(RecipeRef, PinnedRecipe)> {
        self.catalogs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut out = vec![alloy_recipe()];
        out.extend((0..self.filler).map(filler_recipe));
        out
    }
}

fn boot_with_authority(
    recipes: Arc<dyn mush_industry::RecipeAuthority>,
) -> (TestWorld, IndustryFamilies) {
    let cell: InventoryHandleCell = Arc::default();
    let world = TestWorld::with_modules(vec![
        Box::new(mush_economy::EconomyModule::new()),
        Box::new(
            InventoryModule::new()
                .with_quantity_delegates(["industry"])
                .with_handle_cell(Arc::clone(&cell)),
        ),
        Box::new(
            IndustryModule::new()
                .with_inventory(Arc::clone(&cell))
                .with_recipes(recipes),
        ),
    ]);
    let families = world
        .world()
        .module_state::<IndustryHandles>()
        .expect("industry publishes its handles")
        .0
        .clone();
    (world, families)
}

fn boot_with_recipes() -> (TestWorld, IndustryFamilies) {
    boot_with_authority(Arc::new(TestRecipes::default()))
}

/// **E7r1 R1** — the two accounts a work order needs at draft: GOD's budget (which
/// the submit reserves against) and GOD's operating account (which a completed run's
/// capture pays). Returns `(budget, payee)` as raw ids.
fn wo_accounts(world: &mut TestWorld) -> (u64, u64) {
    world.run(GOD, "@instrument/register gold=0");
    let before: Vec<u64> = mine_accounts(world);
    world.run(GOD, &format!("@account/open #{}=gold", GOD.0));
    world.run(GOD, &format!("@account/open #{}=gold", GOD.0));
    let after = mine_accounts(world);
    let fresh: Vec<u64> = after
        .into_iter()
        .filter(|id| !before.contains(id))
        .collect();
    assert_eq!(fresh.len(), 2, "two fresh accounts");
    (fresh[0], fresh[1])
}

fn mine_accounts(world: &TestWorld) -> Vec<u64> {
    world
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .expect("economy store")
        .snapshot()
        .accounts()
        .filter(|a| a.owner == PrincipalId::Object(GOD))
        .map(|a| a.id.0)
        .collect()
}

/// Build a live production schedule + job for one order id, due at `due_at`.
fn seed_schedule(
    engine: &mut Engine,
    families: &IndustryFamilies,
    order: WorkOrderId,
    clock: ClockRef,
    due_at: i64,
) -> ScheduleId {
    let owner = OwnerId::new("industry").unwrap();
    let local = CanonicalId::new(format!("production.{:016x}", order.0)).unwrap();
    let sid = ScheduleId::derive(&owner, &local).unwrap();
    let sch = DurableSchedule::new(DurableScheduleDraft {
        id: sid,
        owner: owner.clone(),
        clock: clock.clone(),
        next_due: ClockStamp { clock, at: due_at },
        cadence: Some(PRODUCTION_CADENCE_SECS),
        generation: 1,
        next_ordinal: 0,
        restart: ScheduleRestartPolicy::Resume,
        catch_up_order: ScheduleCatchUpOrder::OldestFirst,
        max_catch_up: PRODUCTION_MAX_CATCH_UP,
    })
    .unwrap();
    let job = ProductionJob::new(sid, order, GOD, PRODUCTION_UNITS_PER_OCCURRENCE, 1).unwrap();
    apply(engine, families, |txn| {
        txn.insert(&families.schedules, &sid, sch)
            .map_err(IndustryError::from)?;
        txn.insert(&families.jobs, &sid, job)
            .map_err(IndustryError::from)?;
        Ok(())
    })
    .expect("seed schedule + job");
    sid
}

// ---- C1: a recipe can no longer mint output from nothing ----

#[test]
fn vf_c1_a_zero_input_recipe_is_unconstructible() {
    // The pure free-mint floor (D1): a recipe that produces output while consuming
    // no input is minting matter from nothing -- refused at construction, so a
    // forged zero-input/inflated-output pinned cannot be built, drafted or decoded.
    let out = vec![RecipeLine::new(commodity("gold"), 1_000_000).unwrap()];
    assert!(
        PinnedRecipe::new(Vec::new(), out.clone(), BTreeSet::new(), 1).is_err(),
        "a zero-input recipe that mints output is unconstructible"
    );
    // And it cannot be decoded off the wire either (the Deserialize path re-runs
    // the constructor), so the provider's `wo_draft` refuses a forged pinned blob.
    let forged = serde_json::json!({
        "inputs": [],
        "outputs": [{"commodity": "gold", "units": 1_000_000}],
        "requires": [],
        "work_units": 1
    });
    assert!(
        serde_json::from_value::<PinnedRecipe>(forged).is_err(),
        "a zero-input pinned blob is refused on the wire"
    );
    // A one-input recipe is still fine.
    assert!(
        PinnedRecipe::new(
            vec![RecipeLine::new(commodity("ore"), 4).unwrap()],
            out,
            BTreeSet::new(),
            1
        )
        .is_ok()
    );
}

// ---- C2: facility authorization + record-sourced capacity ----

#[test]
fn vf_c2_a_non_owner_cannot_hold_a_facilitys_slots() {
    // Only staff or a principal who `acts_for` the facility's owner may draft
    // there; a stranger gets NO_SUCH_FACILITY byte-for-byte and queues nothing.
    let (mut world, _families) = boot_with_recipes();
    world.run(GOD, "@facility/open 7=2 smelt"); // GOD-owned facility
    let _ = world.engine_mut().take_authoritative_submit();
    // E7r1 R1: a draft now resolves its money accounts at the edit (the budget it
    // reserves and the operating account a completed run pays), so the fixture
    // opens both. The C2 authorization question this test asks is unchanged: a
    // stranger is still refused BEFORE any account is looked at.
    let (budget, _payee) = wo_accounts(&mut world);
    let alice = world.spawn_player("alice"); // a stranger
    let ev = world.run(
        alice,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {budget} 500 600"),
    );
    let stranger = world.seen_by(&ev, alice).join("\n");
    assert!(
        stranger.contains("No such facility"),
        "a non-owner draft is refused byte-identically: {stranger}"
    );
    assert!(
        world.engine_mut().take_authoritative_submit().is_none(),
        "a refused draft queues no submission"
    );
    // The owner (GOD is staff) reaches draft and queues a submission.
    let ev = world.run(
        GOD,
        &format!("@wo/draft tacsim.core/alloy_smelt/1=7 1 {budget} 500 600"),
    );
    let owner = world.seen_by(&ev, GOD).join("\n");
    assert!(
        !owner.contains("No such facility"),
        "the owner is not refused: {owner}"
    );
    assert!(
        world.engine_mut().take_authoritative_submit().is_some(),
        "the owner's draft queued a submission"
    );
}

#[test]
fn vf_c2_capacity_maximum_comes_from_the_record() {
    // (m1) The capacity book's maximum is fixed when the book is first created --
    // from the record-sourced pin the draft command reads -- so a later order
    // pinning a larger `facility_slots` cannot inflate a 1-slot facility to 8.
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let facility = ObjId(9);
    // First reservation creates the book at max 1 (the record's true max).
    apply(engine, &families, |txn| {
        families
            .reserve_slot(txn, facility, 1, &holder(GOD, "wo", 1), &now())
            .map(|_| ())
    })
    .expect("first reserve creates the book at max 1");
    // A second order pinning max 8 cannot raise concurrency above the stored 1.
    let second = apply(engine, &families, |txn| {
        families
            .reserve_slot(txn, facility, 8, &holder(ObjId(2), "wo", 2), &now())
            .map(|_| ())
    });
    assert!(
        second.is_err(),
        "a pinned max of 8 cannot inflate a 1-slot book: {second:?}"
    );
    // Reading with an inflated max still reflects the stored max (1), fully held.
    let free = read(engine, &families, |txn| {
        families.available_slots(txn, facility, 8)
    })
    .expect("read");
    assert_eq!(
        free, 0,
        "the book's stored maximum wins over a client-pinned 8"
    );
}

// ---- R1: a terminal order's schedule is torn down and plans nothing ----

#[test]
fn vf_r1_a_terminal_order_plans_no_more_production() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let order = WorkOrderId(0x1234);
    let ic = engine.world.world_ic_now();
    // A due schedule produces a discovery candidate.
    let sid = seed_schedule(
        engine,
        &families,
        order,
        ic.clock.clone(),
        ic.at.saturating_sub(1),
    );
    let page = engine
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .expect("discover");
    assert!(
        !page.candidates().is_empty(),
        "a live due schedule plans a candidate"
    );
    // The terminal transition tears the schedule and its job down, in one bundle.
    apply(engine, &families, |txn| {
        families.tear_down_production(txn, sid)
    })
    .expect("teardown");
    let page = engine
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .expect("discover");
    assert!(
        page.candidates().is_empty(),
        "a torn-down schedule plans nothing (no post-terminal tick to freeze on)"
    );
    // Both halves are gone.
    let present = read(engine, &families, |txn| {
        (
            txn.get(&families.schedules, &sid).unwrap().is_some(),
            txn.get(&families.jobs, &sid).unwrap().is_some(),
        )
    });
    assert_eq!(present, (false, false), "schedule and job removed");
}

// ---- R2: a generation bump pauses a stale schedule, it does not freeze ----

#[test]
fn vf_r2_a_generation_bump_does_not_freeze_the_scan() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let order = WorkOrderId(0x2222);
    let ic = engine.world.world_ic_now();
    // A Running order and a live schedule on the current generation.
    let running = draft_order(order.0, ObjId(7), 1, 500, 600)
        .acquired(Vec::new(), Vec::new(), None, WorkOrderState::Running)
        .expect("acquire");
    apply(engine, &families, |txn| {
        families.open_work_order(txn, running)
    })
    .expect("open running order");
    seed_schedule(
        engine,
        &families,
        order,
        ic.clock.clone(),
        ic.at + 10 * PRODUCTION_CADENCE_SECS,
    );
    engine
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .expect("discover succeeds before the bump");
    // Open a new world-timeline generation.
    engine.world.set_now(engine.world.now() + 1_000);
    let rate = engine.world.world_timeline().current().rate;
    let epoch = engine
        .world
        .next_world_timeline_epoch(rate)
        .expect("next epoch");
    engine
        .world
        .append_world_timeline_epoch(epoch)
        .expect("append epoch");
    // Before the fix this returned a hard Clock{StaleGeneration} that froze the
    // whole world; now the scan skips the stale (paused) schedule and succeeds.
    let page = engine
        .discover_scheduled_actions(ScheduleDriveCause::Live, None, 32)
        .expect("discover does not freeze after a generation bump");
    assert!(
        page.candidates().is_empty(),
        "the stale-generation schedule is paused (skipped), not a candidate"
    );
}

// ---- R3: the work-order completion family stays bounded ----

#[test]
fn vf_r3_the_completion_family_stays_bounded() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let facility = ObjId(3);
    // Drive more completions than the ring retains, all at one facility.
    for i in 0..(MAX_RETAINED_MARKERS_PER_DEPOSIT as u32 + 5) {
        let request = RequestUid::from_bytes([(i + 1) as u8; 16]);
        let at = 1_000 + i64::from(i);
        let key = mush_industry::marker_id(request, at);
        let marker = WorkOrderMarker::new(
            "industry.workorder.completed",
            WorkOrderId(0x100 + u64::from(i)),
            GOD,
            at,
        )
        .unwrap();
        apply(engine, &families, |txn| {
            families.record_wo_completion(txn, facility, key.clone(), marker)
        })
        .expect("record a completion");
    }
    let retained = engine
        .action_state()
        .scan_family(&families.wo_applied, None, 256)
        .expect("scan wo_applied")
        .entries()
        .len();
    assert!(
        retained <= MAX_RETAINED_MARKERS_PER_DEPOSIT,
        "the durable completion family is bounded per facility ({retained} retained) -- \
         the fact mirror never truncates, so industry facts never go dark"
    );
}

// ---- R4: a both-absent capacity pair fails closed ----

#[test]
fn vf_r4_a_both_absent_capacity_pair_fails_closed() {
    let (mut world, families) = boot();
    let facility = ObjId(77);
    {
        let engine = world.engine_mut();
        // Reserve a slot (creates the capacity book) and a live Running order.
        apply(engine, &families, |txn| {
            families
                .reserve_slot(txn, facility, 2, &holder(GOD, "wo", 1), &now())
                .map(|_| ())
        })
        .expect("reserve a slot");
        let running = draft_order(0xB4B4, facility, 1, 500, 600)
            .acquired(Vec::new(), Vec::new(), None, WorkOrderState::Running)
            .expect("acquire");
        apply(engine, &families, |txn| {
            families.open_work_order(txn, running)
        })
        .expect("open running order");
        // Lose BOTH halves of the capacity book while the order survives.
        let key = facility_capacity_key(facility).expect("key");
        apply(engine, &families, |txn| {
            txn.remove(&families.facility_books, &key)
                .map_err(IndustryError::from)?;
            txn.remove(&families.facility_slots, &key)
                .map_err(IndustryError::from)?;
            Ok(())
        })
        .expect("drop the capacity book");
    }
    // Re-run reconcile (boot's sweep): the cross-family check quarantines the
    // facility rather than reading its slots as fully available.
    world.modules_loaded(std::collections::HashMap::new());
    let store = world
        .world()
        .module_state::<IndustryStore>()
        .expect("industry store");
    assert!(
        store.is_facility_quarantined(facility),
        "a live order + a wholly-absent capacity book quarantines the facility"
    );
    assert_eq!(
        store.facility_quarantine(facility),
        Some(FacilityQuarantine::CapacityLost),
        "the quarantine reason is the lost capacity book"
    );
}

// ---- B2: @wo/list carries an honest continuation cursor ----

#[test]
fn vf_b2_wo_list_pages_to_exhaustion() {
    let (mut world, families) = boot();
    {
        let engine = world.engine_mut();
        // More viewable orders than one page of rows: the row budget must yield a
        // cursor rather than silently dropping the remainder.
        for i in 0..(mush_industry::MAX_LIST_ROWS as u64 + 6) {
            let order = draft_order(0x5000 + i, ObjId(7), 1, 500, 600);
            apply(engine, &families, |txn| {
                families.open_work_order(txn, order)
            })
            .expect("open order");
        }
    }
    // GOD is staff, so every order is viewable.
    let ev = world.run(GOD, "@wo/list");
    let out = world.seen_by(&ev, GOD).join("\n");
    assert!(
        out.contains("(more: @wo/list ="),
        "a full page hands back a continuation cursor: {out}"
    );
    assert!(
        !out.contains("(no work orders)"),
        "the viewer is not falsely told there are none"
    );
}

// ---- B3: @facility/list signals truncation ----

#[test]
fn vf_b3_facility_list_signals_truncation() {
    let (mut world, _families) = boot();
    {
        let engine = world.engine_mut();
        let store = engine
            .world
            .module_state_mut::<IndustryStore>()
            .expect("store");
        for i in 0..(mush_industry::MAX_LIST_ROWS as u32 + 1) {
            store.put_facility(
                FacilityRecord::new(
                    ObjId(1_000 + i),
                    PrincipalId::Object(GOD),
                    capabilities(&["smelt"]),
                    1,
                )
                .unwrap(),
            );
        }
    }
    let ev = world.run(GOD, "@facility/list");
    let out = world.seen_by(&ev, GOD).join("\n");
    assert!(
        out.contains("(more: @facility/list ="),
        "the 65th facility is reachable through a continuation, never silently absent: {out}"
    );
}

// ---- B4: staff may inspect any work order ----

#[test]
fn vf_b4_staff_may_inspect_any_work_order() {
    let (mut world, families) = boot();
    let alice = world.spawn_player("alice");
    let bob = world.spawn_player("bob");
    let (recipe, pinned) = alloy_recipe();
    let order = WorkOrder::draft(
        WorkOrderId(0xB4),
        PrincipalId::Object(alice),
        PrincipalId::Object(alice),
        ObjId(7),
        2,
        1,
        1,
        recipe,
        pinned,
        1,
        AccountId(1),
        Some(endpoint(500)),
        DeliveryTarget::Endpoint {
            endpoint: endpoint(600),
        },
        EconomicVisibility::Parties,
    )
    .expect("draft");
    let id = order.id().to_string();
    {
        let engine = world.engine_mut();
        apply(engine, &families, |txn| {
            families.open_work_order(txn, order)
        })
        .expect("open");
    }
    // Staff (GOD) inspect an order they neither requested nor benefit from.
    let ev = world.run(GOD, &format!("@wo/show {id}"));
    let staff = world.seen_by(&ev, GOD).join("\n");
    assert!(
        !staff.contains("No such work order") && staff.contains("facility #7"),
        "staff inspect any work order: {staff}"
    );
    // A non-party non-staff stranger still gets the byte-identical refusal.
    let ev = world.run(bob, &format!("@wo/show {id}"));
    let stranger = world.seen_by(&ev, bob).join("\n");
    assert!(
        stranger.contains("No such work order"),
        "a genuine non-viewer still gets NO_SUCH_WORK_ORDER: {stranger}"
    );
}

// ---- B5: @wo/set is not silently the list ----

#[test]
fn vf_b5_wo_set_is_not_silently_list() {
    let (mut world, _families) = boot();
    let ev = world.run(GOD, "@wo/set 0/x");
    let out = world.seen_by(&ev, GOD).join("\n");
    assert!(
        out.contains("not available"),
        "@wo/set errors explicitly: {out}"
    );
    assert!(
        !out.contains("(no work orders)"),
        "@wo/set does not silently run the work-order list"
    );
}

// ============================================================================
// ---- WS02-S4: the work-order draft offer's facility gate and its books ----
//
// Before this, `IndustryVerb::WoDraft`'s projection declared thirteen free
// parameters and gated nothing: the command's owner/`acts_for` check, its
// quarantine refusal and its per-requester cap were all reachable only by typing
// `@wo/draft`, and a direct offer submission met none of them. These pin the
// three layers that replaced that — the offer that refuses to exist, the books
// that ride the fixed parameters, and the prepare that re-derives the drafter's
// authority from them.
// ============================================================================

/// The offer id `industry_wo_draft` is projected to this actor under, if at all.
/// The binding hashes the fixed parameters, so the id **is** the books' identity.
fn draft_offer_id(world: &mut TestWorld, actor: ObjId) -> Option<mush_core::ActionOfferId> {
    world.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(mush_core::OfferSecret::new(2, [2; 32]).expect("secret"), [])
            .expect("key ring"),
    );
    let engine = world.engine();
    let action = mush_industry::action_ref(&engine.registry, mush_industry::IndustryVerb::WoDraft)
        .expect("the draft action is registered");
    let query = engine
        .legal_action_query(
            mush_core::PrincipalId::Object { id: actor },
            actor,
            mush_core::PerceivedRef::SelfActor,
        )
        .expect("legal action query");
    let offers = engine
        .project_legal_actions(&query)
        .expect("projection succeeds");
    offers
        .projected_for_action(&action)
        .map(|projected| projected.offer())
}

fn has_draft_offer(world: &mut TestWorld, actor: ObjId) -> bool {
    draft_offer_id(world, actor).is_some()
}

/// Submit `parameters` against this actor's own live `WoDraft` offer and prepare
/// it — the **direct offer route**, which no command gate stands in front of.
fn draft_through_the_offer(
    world: &mut TestWorld,
    actor: ObjId,
    parameters: serde_json::Value,
    token: u8,
) -> Result<(), String> {
    world.engine_mut().set_offer_keys(
        mush_core::OfferKeyRing::new(
            mush_core::OfferSecret::new(2, [2; 32]).map_err(|_| "secret".to_owned())?,
            [],
        )
        .map_err(|_| "key ring".to_owned())?,
    );
    let (action, resolved) = {
        let engine = world.engine();
        let action =
            mush_industry::action_ref(&engine.registry, mush_industry::IndustryVerb::WoDraft)
                .ok_or_else(|| "no draft action".to_owned())?;
        let query = engine
            .legal_action_query(
                mush_core::PrincipalId::Object { id: actor },
                actor,
                mush_core::PerceivedRef::SelfActor,
            )
            .map_err(|error| error.to_string())?;
        let offers = engine
            .project_legal_actions(&query)
            .map_err(|error| error.to_string())?;
        let projected = offers
            .projected_for_action(&action)
            .ok_or_else(|| "no draft offer".to_owned())?;
        let submission = mush_core::OfferSubmission {
            offer: projected.offer(),
            subject: projected.subject().clone(),
            targets: Vec::new(),
            parameters,
        };
        let execution = mush_core::OfferExecutionContext {
            request: RequestUid::from_bytes([token; 16]),
            principal: mush_core::PrincipalId::Object { id: actor },
            actor,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: mush_core::PrincipalId::Object { id: actor },
                    owner: action.key().owner.clone(),
                    namespace: CanonicalId::new(action.key().id.as_str())
                        .map_err(|_| "namespace".to_owned())?,
                },
                1,
                IdempotencyKey::Request(RequestUid::from_bytes([token; 16])),
            ),
            viewer_scope: query.viewer_scope(),
            now: None,
        };
        let resolved = engine
            .resolve_legal_action(&offers, execution, submission)
            .map_err(|error| format!("{error:?}"))?;
        let (action, resolved, _guards) = resolved.into_authoritative_parts();
        (action, resolved)
    };
    world
        .engine_mut()
        .prepare_registered_resolved_authoritative_action(
            &action,
            &resolved,
            CommitToken::from_bytes([token; 16]),
        )
        .map(|_| ())
        .map_err(|error| format!("{error:?}"))
}

/// The parameters a real `@wo/draft` stages, with the facility and its pinned
/// slot count swapped for whatever the caller names.
fn draft_parameters(actor: ObjId, facility: u32, slots: u32) -> serde_json::Value {
    let (_, pinned) = alloy_recipe();
    serde_json::json!({
        "facility": facility,
        "facility_slots": slots,
        "capacity_revision": 1,
        "policy_revision": 1,
        "pack": "tacsim.core",
        "version": 1,
        "recipe": "alloy_smelt",
        "digest": DefinitionDigest::from_bytes([0x11; 32]).to_hex(),
        "quantity": 1,
        "account": 1,
        "source": 500,
        "destination": 600,
        "beneficiary": format!("obj:{}", actor.0),
        "pinned": serde_json::to_string(&pinned).expect("pinned json"),
    })
}

fn install_facility(world: &mut TestWorld, object: u32, owner: ObjId, slots: u32) {
    let store = world
        .engine_mut()
        .world
        .module_state_mut::<IndustryStore>()
        .expect("industry store");
    store.put_facility(
        FacilityRecord::new(
            ObjId(object),
            PrincipalId::Object(owner),
            capabilities(&["smelt"]),
            slots,
        )
        .expect("facility record"),
    );
}

#[test]
fn ws02_s4_an_actor_with_no_facility_is_projected_no_draft_offer() {
    // The fog rule, on the facility axis: an actor who may draft nowhere sees
    // exactly what an actor in a world with no facility at all sees, which is
    // exactly what a verb that does not exist projects — nothing.
    let (mut world, _families) = boot_with_recipes();
    let alice = world.spawn_player("alice");
    assert!(
        !has_draft_offer(&mut world, GOD),
        "no facility exists yet, so not even staff is offered a draft"
    );
    assert!(!has_draft_offer(&mut world, alice));
    world.run(GOD, "@facility/open 7=2 smelt");
    let _ = world.engine_mut().take_authoritative_submit();
    assert!(
        has_draft_offer(&mut world, GOD),
        "the facility's owner is offered the draft"
    );
    assert!(
        !has_draft_offer(&mut world, alice),
        "a stranger is offered nothing — byte-identical to naming a facility that \
         does not exist, so the projection is no existence oracle"
    );
}

#[test]
fn ws02_s4_the_facility_book_excludes_a_rival_and_a_quarantined_facility() {
    let (mut world, families) = boot_with_recipes();
    let alice = world.spawn_player("alice");
    install_facility(&mut world, 21, alice, 2); // hers
    install_facility(&mut world, 22, GOD, 2); // a rival's
    install_facility(&mut world, 23, alice, 2); // hers, about to be quarantined
    {
        // R4's shape: a live order holding a slot at 23 whose capacity book is
        // wholly absent quarantines it on the next reconcile.
        let engine = world.engine_mut();
        let running = draft_order(0x2323, ObjId(23), 1, 500, 600)
            .acquired(Vec::new(), Vec::new(), None, WorkOrderState::Running)
            .expect("acquire");
        apply(engine, &families, |txn| {
            families.open_work_order(txn, running)
        })
        .expect("open running order");
    }
    world.modules_loaded(std::collections::HashMap::new());
    assert!(
        world
            .world()
            .module_state::<IndustryStore>()
            .expect("store")
            .is_facility_quarantined(ObjId(23)),
        "the fixture really quarantined 23"
    );
    let facilities = mush_industry::provider::draftable_facilities(world.engine(), alice, false);
    let offered: Vec<u32> = facilities
        .iter()
        .map(|facility| facility.object().0)
        .collect();
    assert_eq!(
        offered,
        vec![21],
        "the book carries only the facilities alice may draft at, and never a \
         quarantined one"
    );
    let entries = mush_industry::provider::catalog_entries(&TestRecipes::default(), world.world());
    let books =
        mush_industry::provider::wo_offer_books(world.engine(), alice, &facilities, &entries);
    assert!(books.facilities.contains_key("21"));
    assert!(
        !books.facilities.contains_key("22"),
        "a rival's facility is absent"
    );
    assert!(
        !books.facilities.contains_key("23"),
        "a quarantined one is absent"
    );
    assert_eq!(
        books.recipes.len(),
        1,
        "the corridor's one recipe is offered"
    );
    assert!(
        !books.truncated,
        "the corridor's own pack is nowhere near the cap"
    );
}

#[test]
fn ws02_s4_the_per_requester_cap_is_read_off_the_index_not_the_family() {
    let (mut world, families) = boot_with_recipes();
    world.run(GOD, "@facility/open 7=2 smelt");
    let _ = world.engine_mut().take_authoritative_submit();
    assert!(has_draft_offer(&mut world, GOD), "the owner starts offered");
    let mut ids = Vec::new();
    {
        let engine = world.engine_mut();
        for index in 0..mush_industry::MAX_WORK_ORDERS_PER_REQUESTER as u64 {
            let order = draft_order(0x7700 + index, ObjId(7), 1, 500, 600);
            ids.push(order.id());
            apply(engine, &families, |txn| {
                families.open_work_order(txn, order)
            })
            .expect("open order");
        }
    }
    // The sweep's reconcile is what rebuilds the index from the family.
    world.modules_loaded(std::collections::HashMap::new());
    assert_eq!(
        world
            .world()
            .module_state::<IndustryStore>()
            .expect("store")
            .open_work_orders(&PrincipalId::Object(GOD)),
        mush_industry::MAX_WORK_ORDERS_PER_REQUESTER,
        "reconcile counted every non-terminal order"
    );
    assert!(
        !has_draft_offer(&mut world, GOD),
        "at the per-requester cap there is no offer at all"
    );
    // And it really is the INDEX the projection reads, not a walk of the family:
    // empty the index while every one of those orders is still live and open, and
    // the offer comes back.
    {
        let store = world
            .engine_mut()
            .world
            .module_state_mut::<IndustryStore>()
            .expect("store");
        for id in &ids {
            store.note_work_order(&PrincipalId::Object(GOD), *id, false);
        }
        assert_eq!(store.open_requesters(), 0);
    }
    assert!(
        has_draft_offer(&mut world, GOD),
        "the family still holds 16 open orders; the projection consulted the RAM \
         index, which is the whole point of the move"
    );
}

#[test]
fn ws02_s4_two_projections_agree_and_the_catalog_is_walked_once() {
    let counted = Arc::new(TestRecipes::default());
    let catalogs = Arc::clone(&counted.catalogs);
    let (mut world, _families) = boot_with_authority(counted);
    world.run(GOD, "@facility/open 7=2 smelt");
    let _ = world.engine_mut().take_authoritative_submit();
    let first = draft_offer_id(&mut world, GOD).expect("offered");
    let second = draft_offer_id(&mut world, GOD).expect("offered");
    assert_eq!(
        first, second,
        "the offer binding hashes the fixed parameters, so two projections in one \
         IC second agreeing on the id is two byte-identical book sets"
    );
    assert_eq!(
        catalogs.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "the catalog is walked once per publish, not once per projection"
    );
}

#[test]
fn ws02_s4_a_large_catalog_truncates_at_the_cap_and_the_offer_still_builds() {
    let (mut world, _families) = boot_with_authority(Arc::new(TestRecipes::with_filler(400)));
    world.run(GOD, "@facility/open 7=2 smelt");
    let _ = world.engine_mut().take_authoritative_submit();
    let entries =
        mush_industry::provider::catalog_entries(&TestRecipes::with_filler(400), world.world());
    assert_eq!(
        entries.len(),
        401,
        "the fixture really authored 401 recipes"
    );
    assert_eq!(
        entries,
        mush_industry::provider::catalog_entries(&TestRecipes::with_filler(400), world.world()),
        "the catalog reduction is deterministic — the composition root answers a \
         `BTreeMap` walk and this sorts on the rendered key anyway, which is what \
         makes the offer's cache and its byte-identical projections true"
    );
    let facilities = mush_industry::provider::draftable_facilities(world.engine(), GOD, true);
    let books = mush_industry::provider::wo_offer_books(world.engine(), GOD, &facilities, &entries);
    assert_eq!(
        books.recipes.len(),
        mush_industry::provider::MAX_OFFERED_RECIPES,
        "the book is cut at the cap"
    );
    assert!(
        books.truncated,
        "and says so, which is how an operator learns"
    );
    assert!(
        books.recipes.contains_key("tacsim.core/1/alloy_smelt"),
        "the cut is on a sorted key order, not an arbitrary one"
    );
    assert!(
        serde_json::to_vec(&books.recipes).expect("book json").len()
            < mush_core::MAX_PROJECTED_LEGAL_ACTION_BYTES,
        "the book stays well inside the projected-offer bound"
    );
    assert!(
        has_draft_offer(&mut world, GOD),
        "and the offer still builds with 400 recipes in the world"
    );
}

#[test]
fn ws02_s4_an_offer_route_draft_cannot_name_a_facility_it_was_not_offered() {
    // The bypass, attempted: alice has a live `WoDraft` offer of her own (she owns
    // 21), and submits against it a `facility` naming GOD's 22 — the parameter the
    // offer route lets any submitter choose, and the one the command's
    // owner/`acts_for` gate was the only thing that ever checked.
    let (mut world, _families) = boot_with_recipes();
    let alice = world.spawn_player("alice");
    install_facility(&mut world, 21, alice, 2);
    install_facility(&mut world, 22, GOD, 2);
    let forged = draft_through_the_offer(&mut world, alice, draft_parameters(alice, 22, 2), 0x41);
    let message = forged.expect_err("a facility outside her book is refused");
    assert!(
        message.contains("You cannot do that with that work order."),
        "and refused with the byte-identical work-order line: {message}"
    );
    // The same submission naming her own facility, with the record's own slots,
    // goes through — the gate refuses the forgery, not the verb.
    draft_through_the_offer(&mut world, alice, draft_parameters(alice, 21, 2), 0x42)
        .expect("her own facility drafts");
    // And an inflated slot pin at her own facility is refused too: the staleness
    // stamps the submit's drift check reads are the record's, never the client's.
    let inflated = draft_through_the_offer(&mut world, alice, draft_parameters(alice, 21, 8), 0x43);
    assert!(
        inflated.is_err(),
        "a 2-slot facility cannot be pinned as an 8-slot one through the offer route"
    );
}
