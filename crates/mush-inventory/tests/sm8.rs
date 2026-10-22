//! Doc 20 cut SM.8, §7.1 — [`InventoryFamilies::transfer_many_marked`] and
//! [`InventoryFamilies::retain_markers`]: an unload of a hopper that holds two ores moves both
//! into the hold in ONE bundle, one `inventory.transferred` applied marker a move under a
//! caller-derived id, the source's marker ring written once and the destination's pool census
//! staged once (two `transfer_marked` calls on one source would write the ring twice, and
//! `ActionTxn` refuses a second mutation of one key).
//!
//! The rig is `sm6.rs`'s and `ic14.rs`'s: `boot()` composes the module, `apply()` opens the
//! owner-stamped transaction a provider gets and installs it exactly as a receipt does, so an
//! `Err` installs nothing at all. The real clock is the `wall` argument; nothing sleeps.

use mush_core::{CanonicalId, ClockRef, ClockStamp, Engine, GOD};
use mush_economy::VaultId;
use mush_inventory::{
    EndpointRef, InventoryError, InventoryFamilies, InventoryHandles, InventoryModule,
    InventoryRecordError, MARKER_RETENTION_SECS, MAX_MARKER_RING, MarkerRing, PoolCensus,
    derived_marker_id,
};
use mush_testkit::TestWorld;
use serde_json::json;
use smol_str::SmolStr;

/// The real second the first move's offer was drafted.
const T0: i64 = 1_700_000_000;

/// One malformed call: the destination, the moves, and the refusal it must meet.
type Case = (
    EndpointRef,
    Vec<(CanonicalId, u64, SmolStr)>,
    InventoryError,
);

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

fn seed(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef, id: &str, amount: u64) {
    apply(engine, families, |txn| {
        families.credit(txn, at, &commodity(id), amount).map(|_| ())
    })
    .expect("seed credits");
}

fn census(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef) -> Option<u32> {
    read(engine, families, |txn| {
        txn.get(&families.census, &at)
            .expect("census reads")
            .copied()
            .map(PoolCensus::get)
    })
}

fn quantity(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef, id: &str) -> u64 {
    read(engine, families, |txn| {
        families
            .pool(txn, at, &commodity(id))
            .expect("pool reads")
            .map_or(0, |pool| pool.quantity)
    })
}

fn ring(engine: &mut Engine, families: &InventoryFamilies, at: EndpointRef) -> MarkerRing {
    read(engine, families, |txn| {
        txn.get(&families.marker_ring, &at)
            .expect("ring reads")
            .cloned()
            .unwrap_or_default()
    })
}

fn applied(engine: &mut Engine, families: &InventoryFamilies, id: &SmolStr) -> bool {
    read(engine, families, |txn| {
        txn.get(&families.applied, id).expect("reads").is_some()
    })
}

/// A marker id derived the way the unload derives its own (`tacsim::surface::unload::marker`'s
/// shape): the causal value names the move.
fn marker(n: u64, good: &str) -> SmolStr {
    derived_marker_id(1_000, &json!({"tacsim.unload": {"n": n, "good": good}}))
        .expect("a derived id")
}

/// The whole pre-image a refused bundle must leave untouched.
fn image(
    engine: &mut Engine,
    families: &InventoryFamilies,
    from: EndpointRef,
    to: EndpointRef,
    ids: &[SmolStr],
) -> (Vec<u64>, Option<u32>, MarkerRing, Vec<bool>) {
    let mut pools = Vec::new();
    for at in [from, to] {
        for good in ["metal_ore", "ice_ore", "carbon_ore"] {
            pools.push(quantity(engine, families, at, good));
        }
    }
    let census = census(engine, families, to);
    let ring = ring(engine, families, from);
    let applied: Vec<bool> = ids.iter().map(|id| applied(engine, families, id)).collect();
    (pools, census, ring, applied)
}

/// **Two marked moves open two pools in one bundle** (§7.1): a two-commodity source into an
/// endpoint holding neither: both move, the destination's census counts two (staged once), two
/// applied markers stand under the given ids and record each move, and the source's ring holds
/// both ids in order.
#[test]
fn sm8_two_marked_moves_open_two_pools_in_one_bundle() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hopper, hold) = (endpoint(1), endpoint(2));
    seed(engine, &families, hopper, "metal_ore", 12);
    seed(engine, &families, hopper, "ice_ore", 4);
    let (metal, ice) = (marker(1, "metal_ore"), marker(1, "ice_ore"));
    let moved = apply(engine, &families, |txn| {
        families.transfer_many_marked(
            txn,
            hopper,
            hold,
            &[
                (commodity("ice_ore"), 4, ice.clone()),
                (commodity("metal_ore"), 9, metal.clone()),
            ],
            GOD,
            &now(),
            T0,
        )
    })
    .expect("two marked moves land in one bundle");
    assert_eq!(moved.len(), 2);
    assert_eq!((moved[0].moved, moved[1].moved), (4, 9));
    assert_eq!(quantity(engine, &families, hold, "ice_ore"), 4);
    assert_eq!(quantity(engine, &families, hold, "metal_ore"), 9);
    assert_eq!(quantity(engine, &families, hopper, "ice_ore"), 0);
    assert_eq!(quantity(engine, &families, hopper, "metal_ore"), 3);
    assert_eq!(census(engine, &families, hold), Some(2));
    for (id, good, qty) in [(&ice, "ice_ore", 4), (&metal, "metal_ore", 9)] {
        let record = read(engine, &families, |txn| {
            txn.get(&families.applied, id)
                .expect("reads")
                .cloned()
                .expect("the marker stands")
        });
        let value = serde_json::to_value(&record).expect("encodes");
        assert_eq!(value["commodity"], json!(good), "{value}");
        assert_eq!(value["quantity"], json!(qty), "{value}");
    }
    let keys: Vec<SmolStr> = ring(engine, &families, hopper)
        .entries()
        .iter()
        .map(|entry| entry.key.clone())
        .collect();
    assert_eq!(keys, vec![ice, metal], "both ids, in the moves' order");
}

/// **A marker already applied refuses the whole bundle and writes nothing** (the replay guard,
/// `transfer_marked`'s rule): a second bundle reusing one id answers `AlreadyExists`; pools,
/// census, ring and markers are the pre-image.
#[test]
fn sm8_a_marker_already_applied_refuses_and_writes_nothing() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hopper, hold) = (endpoint(1), endpoint(2));
    seed(engine, &families, hopper, "metal_ore", 12);
    seed(engine, &families, hopper, "ice_ore", 4);
    let first = marker(1, "metal_ore");
    apply(engine, &families, |txn| {
        families.transfer_many_marked(
            txn,
            hopper,
            hold,
            &[(commodity("metal_ore"), 2, first.clone())],
            GOD,
            &now(),
            T0,
        )
    })
    .expect("the first move lands");
    let fresh = marker(2, "ice_ore");
    let ids = [first.clone(), fresh.clone()];
    let before = image(engine, &families, hopper, hold, &ids);
    let again = apply(engine, &families, |txn| {
        families.transfer_many_marked(
            txn,
            hopper,
            hold,
            &[
                (commodity("ice_ore"), 1, fresh.clone()),
                (commodity("metal_ore"), 1, first.clone()),
            ],
            GOD,
            &now(),
            T0 + 1,
        )
    });
    assert_eq!(again, Err(InventoryError::AlreadyExists));
    assert_eq!(image(engine, &families, hopper, hold, &ids), before);
}

/// **Every malformed list refuses before anything is staged**: the same endpoint, an empty list,
/// a zero amount, a commodity named twice, a marker named twice, and an id that is not lowercase
/// hex — each its own error, the pre-image untouched.
#[test]
fn sm8_every_malformed_list_refuses_before_staging() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hopper, hold) = (endpoint(1), endpoint(2));
    seed(engine, &families, hopper, "metal_ore", 12);
    seed(engine, &families, hopper, "ice_ore", 4);
    let (a, b) = (marker(1, "metal_ore"), marker(1, "ice_ore"));
    let ids = [a.clone(), b.clone()];
    let before = image(engine, &families, hopper, hold, &ids);
    let cases: Vec<Case> = vec![
        (
            hopper,
            vec![(commodity("metal_ore"), 1, a.clone())],
            InventoryError::SameEndpoint,
        ),
        (
            hold,
            Vec::new(),
            InventoryError::Record(InventoryRecordError::ZeroQuantity),
        ),
        (
            hold,
            vec![
                (commodity("metal_ore"), 1, a.clone()),
                (commodity("ice_ore"), 0, b.clone()),
            ],
            InventoryError::Record(InventoryRecordError::ZeroQuantity),
        ),
        (
            hold,
            vec![
                (commodity("metal_ore"), 1, a.clone()),
                (commodity("metal_ore"), 1, b.clone()),
            ],
            InventoryError::DuplicateCommodity,
        ),
        (
            hold,
            vec![
                (commodity("metal_ore"), 1, a.clone()),
                (commodity("ice_ore"), 1, a.clone()),
            ],
            InventoryError::Identity,
        ),
        (
            hold,
            vec![(commodity("metal_ore"), 1, SmolStr::new("NOT-HEX"))],
            InventoryError::Identity,
        ),
    ];
    for (to, moves, wanted) in cases {
        let refused = apply(engine, &families, |txn| {
            families.transfer_many_marked(txn, hopper, to, &moves, GOD, &now(), T0)
        });
        assert_eq!(refused, Err(wanted), "{moves:?}");
        assert_eq!(image(engine, &families, hopper, hold, &ids), before);
    }
}

/// **A full young ring refuses the whole bundle** (`MarkerRingFull`, fail closed): a source whose
/// ring holds `MAX_MARKER_RING` entries none of which may be evicted refuses a two-move bundle,
/// and nothing — pools, census, ring, markers — is written; the refusal leaves its transaction
/// with nothing staged (decided before the moves).
#[test]
fn sm8_a_full_ring_refuses_the_whole_bundle() {
    let (mut world, families) = boot();
    let engine = world.engine_mut();
    let (hopper, store, hold) = (endpoint(1), endpoint(3), endpoint(2));
    seed(engine, &families, hopper, "carbon_ore", 1_000);
    seed(engine, &families, hopper, "metal_ore", 12);
    seed(engine, &families, hopper, "ice_ore", 4);
    for n in 0..u64::try_from(MAX_MARKER_RING).expect("fits") {
        let id = marker(100 + n, "carbon_ore");
        apply(engine, &families, |txn| {
            families.transfer_many_marked(
                txn,
                hopper,
                store,
                &[(commodity("carbon_ore"), 1, id.clone())],
                GOD,
                &now(),
                T0 + i64::try_from(n).expect("fits"),
            )
        })
        .expect("a young move");
    }
    assert_eq!(ring(engine, &families, hopper).len(), MAX_MARKER_RING);
    let (a, b) = (marker(1, "metal_ore"), marker(1, "ice_ore"));
    let ids = [a.clone(), b.clone()];
    let before = image(engine, &families, hopper, hold, &ids);
    let refused = apply(engine, &families, |txn| {
        families.transfer_many_marked(
            txn,
            hopper,
            hold,
            &[
                (commodity("metal_ore"), 12, a.clone()),
                (commodity("ice_ore"), 4, b.clone()),
            ],
            GOD,
            &now(),
            T0 + 3_600,
        )
    });
    assert_eq!(refused, Err(InventoryError::MarkerRingFull));
    assert_eq!(image(engine, &families, hopper, hold, &ids), before);
    // The cut SM.8 fix round's correctness/F2: the ring's refusal is decided before anything is
    // staged, so a caller may answer it as an outcome — the transaction it leaves holds no write
    // (its prepare refuses `EmptyTransaction`).
    let prepared = {
        let mut txn = engine
            .action_transaction(&families.pools)
            .expect("inventory transaction");
        let refused = families.transfer_many_marked(
            &mut txn,
            hopper,
            hold,
            &[
                (commodity("metal_ore"), 12, a.clone()),
                (commodity("ice_ore"), 4, b.clone()),
            ],
            GOD,
            &now(),
            T0 + 3_600,
        );
        assert_eq!(refused, Err(InventoryError::MarkerRingFull));
        txn.prepare()
            .map(|_| ())
            .map_err(|error| format!("{error:?}"))
    };
    assert!(
        matches!(&prepared, Err(error) if error.contains("EmptyTransaction")),
        "a full ring stages nothing: {prepared:?}"
    );
}

/// **`retain_marker` is `retain_markers` of one** (one ring rule): two worlds whose sources hold
/// forty aged entries take one more move — one through `transfer_marked` (`retain_marker`), the
/// other through `transfer_many_marked` with one move (`retain_markers`) — and end with equal
/// rings and equal evictions from `inventory.applied`.
#[test]
fn sm8_retain_marker_is_retain_markers_of_one() {
    let aged = |single: bool| {
        let (mut world, families) = boot();
        let engine = world.engine_mut();
        let (hopper, hold) = (endpoint(1), endpoint(2));
        seed(engine, &families, hopper, "metal_ore", 1_000);
        let ids: Vec<SmolStr> = (0..40)
            .map(|n| {
                let id = marker(n, "metal_ore");
                apply(engine, &families, |txn| {
                    families.transfer_marked(
                        txn,
                        hopper,
                        hold,
                        &commodity("metal_ore"),
                        1,
                        &id,
                        GOD,
                        &now(),
                        T0 + i64::try_from(n).expect("fits"),
                    )
                })
                .expect("an early move");
                id
            })
            .collect();
        let late = T0 + MARKER_RETENTION_SECS + 3_600;
        let last = marker(9_999, "metal_ore");
        if single {
            apply(engine, &families, |txn| {
                families.transfer_marked(
                    txn,
                    hopper,
                    hold,
                    &commodity("metal_ore"),
                    1,
                    &last,
                    GOD,
                    &now(),
                    late,
                )
            })
            .expect("the late move");
        } else {
            apply(engine, &families, |txn| {
                families.transfer_many_marked(
                    txn,
                    hopper,
                    hold,
                    &[(commodity("metal_ore"), 1, last.clone())],
                    GOD,
                    &now(),
                    late,
                )
            })
            .expect("the late move");
        }
        let still: Vec<bool> = ids
            .iter()
            .chain(std::iter::once(&last))
            .map(|id| applied(engine, &families, id))
            .collect();
        (ring(engine, &families, hopper), still)
    };
    let (one, many) = (aged(true), aged(false));
    assert_eq!(one, many, "the same ring and the same evictions");
    assert!(
        one.1.iter().any(|kept| !kept),
        "the late push evicted aged markers"
    );
}
