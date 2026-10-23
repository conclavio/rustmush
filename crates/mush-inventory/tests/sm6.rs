//! Doc 20 cut SM.6, lead ruling 5 — [`InventoryFamilies::transfer_many`]: one
//! bundle may open any number of new pools at one endpoint, because the
//! destination's pool census is staged **once** for every new pool (SM.3b's
//! round F3: two `transfer`s that each open a pool at one endpoint wrote
//! `inventory.census` twice and `ActionTxn::require_fresh_mutation` refused).
//!
//! The rig is `e7c2.rs`'s: `boot()` composes the module, `apply()` opens the
//! owner-stamped transaction a provider gets and installs it exactly as a
//! receipt does, so an `Err` installs nothing at all.

use mush_core::{CanonicalId, Engine};
use mush_economy::VaultId;
use mush_inventory::{
    EndpointRef, InventoryError, InventoryFamilies, InventoryHandles, InventoryModule,
    InventoryRecordError, MAX_POOLS_PER_ENDPOINT, Pool, PoolCensus,
};
use mush_testkit::TestWorld;

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

fn commodity(id: &str) -> CanonicalId {
    CanonicalId::new(id).expect("test commodity id")
}

fn endpoint(id: u64) -> EndpointRef {
    EndpointRef(VaultId(id))
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
            Ok(value) => (Ok(value), Some(txn.prepare().expect("the bundle prepares"))),
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

fn census(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef) -> Option<u32> {
    let mut txn = engine
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    txn.get(&families.census, &at)
        .expect("census reads")
        .copied()
        .map(PoolCensus::get)
}

fn pool(
    engine: &mut Engine,
    families: &InventoryFamilies,
    at: EndpointRef,
    id: &str,
) -> Option<Pool> {
    let mut txn = engine
        .action_transaction(&families.pools)
        .expect("inventory transaction");
    families
        .pool(&mut txn, at, &commodity(id))
        .expect("pool reads")
}

fn seed(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef, id: &str, amount: u64) {
    apply(engine, families, |txn| {
        families.credit(txn, at, &commodity(id), amount).map(|_| ())
    })
    .expect("seed credits");
}

#[test]
fn sm6_two_new_pools_on_one_endpoint_land_in_one_bundle() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let hopper = endpoint(1);
    let spoil = endpoint(2);
    seed(engine, &families, hopper, "metal_ore", 5);
    seed(engine, &families, hopper, "ice_ore", 3);
    assert_eq!(census(engine, &families, spoil), None);

    let moved = apply(engine, &families, |txn| {
        families.transfer_many(
            txn,
            hopper,
            spoil,
            &[(commodity("metal_ore"), 2), (commodity("ice_ore"), 1)],
        )
    })
    .expect("two new pools on one endpoint land in one bundle");
    assert_eq!(moved.len(), 2);
    assert_eq!(moved[0].moved, 2);
    assert_eq!(moved[1].moved, 1);

    // The destination's census is +2, staged once; both pools are exact and
    // the source is debited by exactly what moved.
    assert_eq!(census(engine, &families, spoil), Some(2));
    assert_eq!(
        pool(engine, &families, spoil, "metal_ore").map(|p| p.quantity),
        Some(2)
    );
    assert_eq!(
        pool(engine, &families, spoil, "ice_ore").map(|p| p.quantity),
        Some(1)
    );
    assert_eq!(
        pool(engine, &families, hopper, "metal_ore").map(|p| p.quantity),
        Some(3)
    );
    assert_eq!(
        pool(engine, &families, hopper, "ice_ore").map(|p| p.quantity),
        Some(2)
    );
    // The source's census never moves on a move out.
    assert_eq!(census(engine, &families, hopper), Some(2));

    // A second bundle into one existing and one new pool counts only the new.
    seed(engine, &families, hopper, "carbon_ore", 4);
    apply(engine, &families, |txn| {
        families.transfer_many(
            txn,
            hopper,
            spoil,
            &[(commodity("metal_ore"), 1), (commodity("carbon_ore"), 4)],
        )
    })
    .expect("one existing and one new pool");
    assert_eq!(census(engine, &families, spoil), Some(3));
    assert_eq!(
        pool(engine, &families, spoil, "metal_ore").map(|p| p.quantity),
        Some(3)
    );
}

#[test]
fn sm6_transfer_many_refuses_a_repeated_commodity_and_the_same_endpoint() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let hopper = endpoint(1);
    let spoil = endpoint(2);
    seed(engine, &families, hopper, "metal_ore", 5);

    let twice = apply(engine, &families, |txn| {
        families.transfer_many(
            txn,
            hopper,
            spoil,
            &[(commodity("metal_ore"), 1), (commodity("metal_ore"), 1)],
        )
    });
    assert_eq!(twice, Err(InventoryError::DuplicateCommodity));

    let same = apply(engine, &families, |txn| {
        families.transfer_many(txn, hopper, hopper, &[(commodity("metal_ore"), 1)])
    });
    assert_eq!(same, Err(InventoryError::SameEndpoint));

    let empty = apply(engine, &families, |txn| {
        families.transfer_many(txn, hopper, spoil, &[])
    });
    assert_eq!(
        empty,
        Err(InventoryError::Record(InventoryRecordError::ZeroQuantity))
    );
    let zero = apply(engine, &families, |txn| {
        families.transfer_many(txn, hopper, spoil, &[(commodity("metal_ore"), 0)])
    });
    assert_eq!(
        zero,
        Err(InventoryError::Record(InventoryRecordError::ZeroQuantity))
    );

    // Nothing moved.
    assert_eq!(census(engine, &families, spoil), None);
    assert_eq!(
        pool(engine, &families, hopper, "metal_ore").map(|p| p.quantity),
        Some(5)
    );
}

#[test]
fn sm6_transfer_many_past_the_endpoint_cap_writes_nothing() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let hopper = endpoint(1);
    let spoil = endpoint(2);
    // 63 pools at the spoil: one short of the cap.
    for n in 0..(MAX_POOLS_PER_ENDPOINT - 1) {
        seed(engine, &families, spoil, &format!("filler_{n:02}"), 1);
    }
    assert_eq!(
        census(engine, &families, spoil),
        Some(u32::try_from(MAX_POOLS_PER_ENDPOINT - 1).expect("cap fits"))
    );
    seed(engine, &families, hopper, "metal_ore", 5);
    seed(engine, &families, hopper, "ice_ore", 5);
    let before = (
        census(engine, &families, spoil),
        pool(engine, &families, hopper, "metal_ore"),
        pool(engine, &families, hopper, "ice_ore"),
        pool(engine, &families, spoil, "metal_ore"),
        pool(engine, &families, spoil, "ice_ore"),
    );

    let refused = apply(engine, &families, |txn| {
        families.transfer_many(
            txn,
            hopper,
            spoil,
            &[(commodity("metal_ore"), 1), (commodity("ice_ore"), 1)],
        )
    });
    assert_eq!(
        refused,
        Err(InventoryError::Record(
            InventoryRecordError::EndpointPools {
                max: MAX_POOLS_PER_ENDPOINT
            }
        ))
    );
    let after = (
        census(engine, &families, spoil),
        pool(engine, &families, hopper, "metal_ore"),
        pool(engine, &families, hopper, "ice_ore"),
        pool(engine, &families, spoil, "metal_ore"),
        pool(engine, &families, spoil, "ice_ore"),
    );
    assert_eq!(before, after, "the pre-image is untouched");
    // One new pool still fits.
    apply(engine, &families, |txn| {
        families.transfer_many(txn, hopper, spoil, &[(commodity("metal_ore"), 1)])
    })
    .expect("the 64th pool fits");
    assert_eq!(
        census(engine, &families, spoil),
        Some(u32::try_from(MAX_POOLS_PER_ENDPOINT).expect("cap fits"))
    );
}

#[test]
fn sm6_a_single_transfer_still_counts_its_new_pool() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let hopper = endpoint(1);
    let spoil = endpoint(2);
    seed(engine, &families, hopper, "metal_ore", 5);
    seed(engine, &families, hopper, "ice_ore", 5);
    apply(engine, &families, |txn| {
        families.transfer(txn, hopper, spoil, &commodity("metal_ore"), 2)
    })
    .expect("one transfer");
    assert_eq!(census(engine, &families, spoil), Some(1));
    // The old path's defect stands for `transfer`: two new pools at one
    // endpoint in one bundle still refuse (that is why `transfer_many` exists).
    let twice = apply(engine, &families, |txn| {
        families.transfer(txn, hopper, endpoint(3), &commodity("metal_ore"), 1)?;
        families.transfer(txn, hopper, endpoint(3), &commodity("ice_ore"), 1)
    });
    assert!(twice.is_err(), "two transfers writing one census refuse");
    assert_eq!(census(engine, &families, endpoint(3)), None);
    assert_eq!(census(engine, &families, spoil), Some(1));
}
