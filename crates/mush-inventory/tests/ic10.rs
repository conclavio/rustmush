//! Doc 18 cut IC10 — the inventory half of a sale into a standing bid:
//! [`derived_marker_id`] (the caller-derived, offer-time-nameable applied-marker
//! id) and [`InventoryFamilies::transfer_marked`] (a transfer that writes its own
//! `inventory.transferred` marker under that id, the replay guard included).
//!
//! The rig is `e7c2.rs`'s: `boot()` composes the module, `apply()` opens the
//! owner-stamped transaction a provider gets and installs it exactly as an exact
//! receipt does, so every "nothing written" assertion means what it says.

use mush_core::{ClockRef, ClockStamp, Engine, ObjId};
use mush_economy::{ExternalRef, FactProvider, VaultId};
use mush_inventory::{
    EndpointRef, Grade, InventoryError, InventoryFactProvider, InventoryFamilies, InventoryHandles,
    InventoryModule, Lineage, Lot, LotId, MARKER_CAUSAL_MAX_BYTES, derived_marker_id,
};
use mush_testkit::TestWorld;
use serde_json::json;
use smol_str::SmolStr;

fn boot() -> (TestWorld, InventoryFamilies) {
    let world = TestWorld::with_modules(vec![Box::new(InventoryModule::new())]);
    let families = world
        .world()
        .module_state::<InventoryHandles>()
        .expect("inventory publishes its handles")
        .0
        .clone();
    (world, families)
}

fn commodity(id: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(id).expect("test commodity id")
}

fn endpoint(id: u64) -> EndpointRef {
    EndpointRef(VaultId(id))
}

/// The offer's real-clock stamp every marked transfer here carries (doc 18
/// cut IC14 row 1: `transfer_marked`'s `wall`).
const T_WALL: i64 = 1_700_000_000;

fn now() -> ClockStamp {
    ClockStamp {
        clock: ClockRef::WorldIc { generation: 1 },
        at: 1_000,
    }
}

fn apply<T>(
    engine: &mut Engine,
    families: &InventoryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> Result<T, InventoryError>,
) -> Result<T, InventoryError> {
    let (result, prepared) = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
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
    families: &InventoryFamilies,
    body: impl FnOnce(&mut mush_core::ActionTxn<'_>) -> T,
) -> T {
    let mut txn = engine
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    body(&mut txn)
}

fn balance(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef) -> u64 {
    read(engine, families, |txn| {
        families
            .pool(txn, at, &commodity("metals"))
            .expect("pool reads")
            .map_or(0, |pool| pool.quantity)
    })
}

/// A rooted pool: one Origin lot poured in, so the pool has a lineage root a
/// transfer can carry.
fn seed_rooted(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef, amount: u64) {
    let lot = LotId(77);
    let record = Lot::new(
        lot,
        at,
        commodity("metals"),
        amount,
        Grade::new(0).expect("grade"),
        Lineage::Origin {
            source: ExternalRef {
                provider: "test".into(),
                kind: "seed".into(),
                id: "1".into(),
                revision: None,
            },
        },
        0,
    )
    .expect("seed lot");
    apply(engine, families, |txn| {
        let next = mush_inventory::LotCensus::default().checked_move(1)?;
        txn.insert(&families.lot_census, &at, next)
            .map_err(InventoryError::from)?;
        txn.insert(&families.lots, &lot, record)
            .map_err(InventoryError::from)
    })
    .expect("seed lot installs");
    apply(engine, families, |txn| families.dissolve(txn, at, lot)).expect("dissolve");
}

fn causal(contract: u64, listing: u64, quantity: u64) -> serde_json::Value {
    json!({"kind": "market.deliver", "contract": contract, "listing": listing, "quantity": quantity})
}

fn is_lower_hex(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[test]
fn ic10_derived_marker_ids_are_stable_hex_and_time_ordered() {
    // The widest causal value the market ever derives from, and a small one.
    let wide = causal(u64::MAX, u64::MAX, u64::MAX);
    let small = causal(1, 2, 3);
    for value in [&wide, &small] {
        let id = derived_marker_id(1_000, value).expect("derives");
        assert_eq!(id.len(), 80, "16 hex of time + 64 hex of digest");
        assert!(is_lower_hex(&id), "{id}");
        assert_eq!(id, derived_marker_id(1_000, value).expect("derives again"));
    }
    // A later IC second sorts after an earlier one (the mirror keeps the newest).
    let early = derived_marker_id(-5, &small).expect("early");
    let mid = derived_marker_id(1_000, &small).expect("mid");
    let late = derived_marker_id(1_001, &small).expect("late");
    assert!(early < mid && mid < late);
    // Another causal input is another id.
    assert_ne!(
        derived_marker_id(1_000, &causal(1, 2, 4)).expect("other"),
        mid
    );
    // The cap is on the canonical INPUT, never a digest length.
    let fat = json!({"kind": "x".repeat(MARKER_CAUSAL_MAX_BYTES)});
    assert_eq!(
        derived_marker_id(1_000, &fat),
        Err(InventoryError::Identity)
    );
}

#[test]
fn ic10_a_marked_transfer_moves_carries_lineage_and_writes_its_marker() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hold, store) = (endpoint(1), endpoint(2));
    seed_rooted(engine, &families, hold, 50);
    let marker = derived_marker_id(1_000, &causal(9, 4, 20)).expect("id");
    let actor = ObjId(42);
    let moved = apply(engine, &families, |txn| {
        families.transfer_marked(
            txn,
            hold,
            store,
            &commodity("metals"),
            20,
            &marker,
            actor,
            &now(),
            T_WALL,
        )
    })
    .expect("marked transfer");
    assert_eq!(moved.moved, 20);
    assert_eq!(balance(engine, &families, hold), 30);
    assert_eq!(balance(engine, &families, store), 20);
    let (source_root, dest_root, record) = read(engine, &families, |txn| {
        let source = families
            .tranche_for(txn, hold, &commodity("metals"))
            .expect("reads")
            .expect("source root");
        let dest = families
            .tranche_for(txn, store, &commodity("metals"))
            .expect("reads")
            .expect("destination root");
        let record = txn
            .get(&families.applied, &marker)
            .expect("reads")
            .cloned()
            .expect("the marker was written");
        (source, dest, record)
    });
    // The destination root inherits the source root, carrying exactly 20.
    assert_eq!(dest_root.1.inherited(), &[source_root.0]);
    assert_eq!(dest_root.1.quantity(), 20);
    assert_eq!(source_root.1.remaining(), 30);
    assert_eq!(record.kind(), mush_inventory::FACT_TRANSFERRED);
    assert_eq!(record.from(), Some(hold));
    assert_eq!(record.to(), Some(store));
    assert_eq!(record.quantity(), 20);
    assert_eq!(record.actor(), actor);
    assert_eq!(record.at(), 1_000);
}

#[test]
fn ic10_a_marked_transfer_refuses_a_replay_a_bad_id_and_one_endpoint() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hold, store) = (endpoint(1), endpoint(2));
    seed_rooted(engine, &families, hold, 50);
    let marker = derived_marker_id(1_000, &causal(9, 4, 20)).expect("id");
    let go = |engine: &mut Engine, from, to, amount, marker: &SmolStr| {
        apply(engine, &families, |txn| {
            families.transfer_marked(
                txn,
                from,
                to,
                &commodity("metals"),
                amount,
                marker,
                ObjId(42),
                &now(),
                T_WALL,
            )
        })
    };
    go(engine, hold, store, 20, &marker).expect("first lands");
    // The replay guard: the same marker again moves nothing.
    assert_eq!(
        go(engine, hold, store, 20, &marker),
        Err(InventoryError::AlreadyExists)
    );
    assert_eq!(balance(engine, &families, hold), 30);
    assert_eq!(balance(engine, &families, store), 20);
    // Not lowercase hex.
    assert_eq!(
        go(engine, hold, store, 1, &SmolStr::new("XYZ")),
        Err(InventoryError::Identity)
    );
    // One endpoint at both ends.
    let other = derived_marker_id(1_000, &causal(9, 4, 1)).expect("id");
    assert_eq!(
        go(engine, hold, hold, 1, &other),
        Err(InventoryError::SameEndpoint)
    );
    // Insufficient: refused, and no marker left behind.
    let short = derived_marker_id(1_000, &causal(9, 4, 31)).expect("id");
    assert!(matches!(
        go(engine, hold, store, 31, &short),
        Err(InventoryError::Book(_) | InventoryError::Insufficient { .. })
    ));
    let written = read(engine, &families, |txn| {
        txn.get(&families.applied, &short).expect("reads").is_some()
    });
    assert!(!written, "a refused transfer writes no marker");
    assert_eq!(balance(engine, &families, hold), 30);
}

#[test]
fn ic10_the_inventory_fact_answers_a_derived_marker() {
    let (mut world, families) = boot();
    let marker = derived_marker_id(1_000, &causal(9, 4, 20)).expect("id");
    {
        let engine = world.engine_mut();
        seed_rooted(engine, &families, endpoint(1), 50);
        apply(engine, &families, |txn| {
            families.transfer_marked(
                txn,
                endpoint(1),
                endpoint(2),
                &commodity("metals"),
                20,
                &marker,
                ObjId(42),
                &now(),
                T_WALL,
            )
        })
        .expect("marked transfer");
    }
    // The mirror is refreshed at boot and on the sweep tick (E7c2 fold B4).
    world.modules_loaded(std::collections::HashMap::new());
    let provider = InventoryFactProvider;
    for revision in [None, Some(1)] {
        let state = provider
            .resolve(
                world.world(),
                mush_inventory::FACT_TRANSFERRED,
                &marker,
                revision,
            )
            .expect("the derived marker answers");
        assert!(state.satisfied);
        assert_eq!(state.actor, Some(ObjId(42)));
    }
    assert!(
        provider
            .resolve(
                world.world(),
                mush_inventory::FACT_TRANSFERRED,
                &marker,
                Some(2)
            )
            .is_none()
    );
    assert!(
        provider
            .resolve(
                world.world(),
                mush_inventory::FACT_LOT_CREATED,
                &marker,
                None
            )
            .is_none()
    );
}
